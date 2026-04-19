use astrcode_core::{
    ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig, ResolvedSubagentContextOverrides,
    ResolvedTurnEnvelope,
};
use astrcode_kernel::CapabilityRouter;

use super::{
    BuildSurfaceInput, FreshChildGovernanceInput, GovernanceBusyPolicy, ResolvedGovernanceSurface,
    ResumedChildGovernanceInput, RootGovernanceInput, SessionGovernanceInput,
};
use crate::{
    AgentSessionPort, AppKernelPort, ApplicationError, CompiledModeEnvelope, ComposerResolvedSkill,
    ExecutionControl, ModeCatalog, compile_mode_envelope, compile_mode_envelope_for_child,
};

#[derive(Debug, Clone)]
pub struct GovernanceSurfaceAssembler {
    mode_catalog: ModeCatalog,
}

impl GovernanceSurfaceAssembler {
    pub fn new(mode_catalog: ModeCatalog) -> Self {
        Self { mode_catalog }
    }

    pub fn runtime_with_control(
        &self,
        mut runtime: ResolvedRuntimeConfig,
        control: Option<&ExecutionControl>,
        allow_manual_compact: bool,
    ) -> Result<ResolvedRuntimeConfig, ApplicationError> {
        if let Some(control) = control {
            control.validate()?;
            if let Some(max_steps) = control.max_steps {
                runtime.max_steps = max_steps as usize;
            }
            if !allow_manual_compact && control.manual_compact.is_some() {
                return Err(ApplicationError::InvalidArgument(
                    "manualCompact is not valid for prompt submission".to_string(),
                ));
            }
        }
        Ok(runtime)
    }

    pub fn build_submission_skill_declaration(
        &self,
        skill: &ComposerResolvedSkill,
        user_prompt: Option<&str>,
    ) -> astrcode_core::PromptDeclaration {
        let mut content = format!(
            "The user explicitly selected the `{}` skill for this turn.\n\nSelected skill:\n- id: \
             {}\n- description: {}\n\nTurn contract:\n- Call the `Skill` tool for `{}` before \
             continuing.\n- Treat the user's message as the task-specific instruction for this \
             skill.\n- If the user message is empty, follow the skill's default workflow and ask \
             only if blocked.\n- Do not silently substitute a different skill unless `{}` is \
             unavailable.",
            skill.id,
            skill.id,
            skill.description.trim(),
            skill.id,
            skill.id
        );
        if let Some(user_prompt) = user_prompt.map(str::trim).filter(|value| !value.is_empty()) {
            content.push_str(&format!("\n- User prompt focus: {user_prompt}"));
        }
        astrcode_core::PromptDeclaration {
            block_id: format!("submission.skill.{}", skill.id),
            title: format!("Selected Skill: {}", skill.id),
            content,
            render_target: astrcode_core::PromptDeclarationRenderTarget::System,
            layer: astrcode_core::SystemPromptLayer::Dynamic,
            kind: astrcode_core::PromptDeclarationKind::ExtensionInstruction,
            priority_hint: Some(590),
            always_include: true,
            source: astrcode_core::PromptDeclarationSource::Builtin,
            capability_name: None,
            origin: Some(format!("skill-slash:{}", skill.id)),
        }
    }

    fn compile_mode_surface(
        &self,
        kernel: &dyn AppKernelPort,
        mode_id: &astrcode_core::ModeId,
        extra_prompt_declarations: Vec<astrcode_core::PromptDeclaration>,
    ) -> Result<CompiledModeEnvelope, ApplicationError> {
        let spec = self.mode_catalog.get(mode_id).ok_or_else(|| {
            ApplicationError::InvalidArgument(format!("unknown mode '{}'", mode_id))
        })?;
        compile_mode_envelope(
            kernel.gateway().capabilities(),
            &spec,
            extra_prompt_declarations,
        )
        .map_err(ApplicationError::from)
    }

    fn compile_child_mode_surface(
        &self,
        kernel: &dyn AppKernelPort,
        mode_id: &astrcode_core::ModeId,
        parent_allowed_tools: &[String],
        capability_grant: Option<&astrcode_core::SpawnCapabilityGrant>,
    ) -> Result<CompiledModeEnvelope, ApplicationError> {
        let spec = self.mode_catalog.get(mode_id).ok_or_else(|| {
            ApplicationError::InvalidArgument(format!("unknown mode '{}'", mode_id))
        })?;
        compile_mode_envelope_for_child(
            kernel.gateway().capabilities(),
            &spec,
            parent_allowed_tools,
            capability_grant,
        )
        .map_err(ApplicationError::from)
    }

    fn build_surface(
        &self,
        input: BuildSurfaceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let BuildSurfaceInput {
            session_id,
            turn_id,
            working_dir,
            profile,
            compiled,
            runtime,
            requested_busy_policy,
            resolved_overrides,
            injected_messages,
            leading_prompt_declaration,
        } = input;
        let mut prompt_declarations = compiled.envelope.prompt_declarations.clone();
        if let Some(leading) = leading_prompt_declaration {
            prompt_declarations.insert(0, leading);
        }
        prompt_declarations.extend(super::prompt::collaboration_prompt_declarations(
            &compiled.envelope.allowed_tools,
            runtime.agent.max_subrun_depth,
            runtime.agent.max_spawn_per_turn,
        ));
        let busy_policy = super::policy::resolve_busy_policy(
            compiled.envelope.submit_busy_policy,
            requested_busy_policy,
        );
        let surface = ResolvedGovernanceSurface {
            mode_id: compiled.envelope.mode_id.clone(),
            runtime: runtime.clone(),
            capability_router: compiled.capability_router,
            prompt_declarations,
            resolved_limits: ResolvedExecutionLimitsSnapshot {
                allowed_tools: compiled.envelope.allowed_tools.clone(),
                max_steps: Some(runtime.max_steps as u32),
            },
            resolved_overrides,
            injected_messages,
            policy_context: super::policy::build_policy_context(
                &session_id,
                &turn_id,
                &working_dir,
                &profile,
                &compiled.envelope,
            ),
            collaboration_policy: super::collaboration_policy_context(&runtime),
            approval: super::policy::default_approval_pipeline(
                &session_id,
                &turn_id,
                &compiled.envelope,
            ),
            governance_revision: super::GOVERNANCE_POLICY_REVISION.to_string(),
            busy_policy,
            diagnostics: compiled.envelope.diagnostics.clone(),
        };
        surface.validate()?;
        Ok(surface)
    }

    pub fn session_surface(
        &self,
        kernel: &dyn AppKernelPort,
        input: SessionGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let runtime = self.runtime_with_control(input.runtime, input.control.as_ref(), false)?;
        let compiled =
            self.compile_mode_surface(kernel, &input.mode_id, input.extra_prompt_declarations)?;
        self.build_surface(BuildSurfaceInput {
            session_id: input.session_id,
            turn_id: input.turn_id,
            working_dir: input.working_dir,
            profile: input.profile,
            compiled,
            runtime,
            requested_busy_policy: input.busy_policy,
            resolved_overrides: None,
            injected_messages: Vec::new(),
            leading_prompt_declaration: None,
        })
    }

    pub fn root_surface(
        &self,
        kernel: &dyn AppKernelPort,
        input: RootGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        self.session_surface(
            kernel,
            SessionGovernanceInput {
                session_id: input.session_id,
                turn_id: input.turn_id,
                working_dir: input.working_dir,
                profile: input.profile,
                mode_id: input.mode_id,
                runtime: input.runtime,
                control: input.control,
                extra_prompt_declarations: Vec::new(),
                busy_policy: GovernanceBusyPolicy::BranchOnBusy,
            },
        )
    }

    pub async fn fresh_child_surface(
        &self,
        kernel: &dyn AppKernelPort,
        session_runtime: &dyn AgentSessionPort,
        input: FreshChildGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let compiled = self.compile_child_mode_surface(
            kernel,
            &input.mode_id,
            &input.parent_allowed_tools,
            input.capability_grant.as_ref(),
        )?;
        let resolved_overrides = ResolvedSubagentContextOverrides {
            fork_mode: compiled.envelope.fork_mode.clone(),
            ..ResolvedSubagentContextOverrides::default()
        };
        let injected_messages = super::resolve_inherited_parent_messages(
            session_runtime,
            &input.session_id,
            &resolved_overrides,
        )
        .await?;
        let delegation = super::build_delegation_metadata(
            input.description.as_str(),
            input.task.as_str(),
            &ResolvedExecutionLimitsSnapshot {
                allowed_tools: compiled.envelope.allowed_tools.clone(),
                max_steps: Some(input.runtime.max_steps as u32),
            },
            compiled.envelope.child_policy.restricted,
        );
        self.build_surface(BuildSurfaceInput {
            session_id: input.session_id,
            turn_id: input.turn_id,
            working_dir: input.working_dir,
            profile: "subagent".to_string(),
            compiled,
            runtime: input.runtime,
            requested_busy_policy: input.busy_policy,
            resolved_overrides: Some(resolved_overrides),
            injected_messages,
            leading_prompt_declaration: Some(super::build_fresh_child_contract(&delegation)),
        })
    }

    pub fn resumed_child_surface(
        &self,
        kernel: &dyn AppKernelPort,
        input: ResumedChildGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let mut runtime = input.runtime;
        if let Some(max_steps) = input.resolved_limits.max_steps {
            runtime.max_steps = max_steps as usize;
        }
        let compiled = self.compile_mode_surface(kernel, &input.mode_id, Vec::new())?;
        let allowed_tools = if input.allowed_tools.is_empty() {
            if input.resolved_limits.allowed_tools.is_empty() {
                compiled.envelope.allowed_tools.clone()
            } else {
                input.resolved_limits.allowed_tools.clone()
            }
        } else {
            input.allowed_tools
        };
        let delegation = input.delegation.unwrap_or_else(|| {
            super::build_delegation_metadata(
                "",
                input.message.as_str(),
                &input.resolved_limits,
                false,
            )
        });
        let compiled = CompiledModeEnvelope {
            capability_router: if allowed_tools == compiled.envelope.allowed_tools {
                compiled.capability_router
            } else if allowed_tools.is_empty() {
                Some(CapabilityRouter::empty())
            } else {
                Some(
                    kernel
                        .gateway()
                        .capabilities()
                        .subset_for_tools_checked(&allowed_tools)
                        .map_err(|error| ApplicationError::InvalidArgument(error.to_string()))?,
                )
            },
            envelope: ResolvedTurnEnvelope {
                allowed_tools: allowed_tools.clone(),
                ..compiled.envelope
            },
            spec: compiled.spec,
        };
        self.build_surface(BuildSurfaceInput {
            session_id: input.session_id,
            turn_id: input.turn_id,
            working_dir: input.working_dir,
            profile: "subagent".to_string(),
            compiled,
            runtime,
            requested_busy_policy: input.busy_policy,
            resolved_overrides: None,
            injected_messages: Vec::new(),
            leading_prompt_declaration: Some(super::build_resumed_child_contract(
                &delegation,
                input.message.as_str(),
                input.context.as_deref(),
            )),
        })
    }

    pub fn tool_collaboration_context(
        &self,
        runtime: ResolvedRuntimeConfig,
        session_id: String,
        turn_id: String,
        parent_agent_id: Option<String>,
        source_tool_call_id: Option<String>,
        mode_id: astrcode_core::ModeId,
    ) -> super::ToolCollaborationGovernanceContext {
        super::ToolCollaborationGovernanceContext::new(
            super::ToolCollaborationGovernanceContextInput {
                runtime: runtime.clone(),
                session_id,
                turn_id,
                parent_agent_id,
                source_tool_call_id,
                policy: super::collaboration_policy_context(&runtime),
                governance_revision: super::GOVERNANCE_POLICY_REVISION.to_string(),
                mode_id,
            },
        )
    }

    pub fn mode_catalog(&self) -> &ModeCatalog {
        &self.mode_catalog
    }
}

impl Default for GovernanceSurfaceAssembler {
    fn default() -> Self {
        Self::new(
            crate::mode::builtin_mode_catalog()
                .expect("builtin governance mode catalog should build"),
        )
    }
}
