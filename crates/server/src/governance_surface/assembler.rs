//! 治理面装配器。
//!
//! `GovernanceSurfaceAssembler` 是治理面子域的 bind owner：先消费 mode compiler 的产物，
//! 再把 runtime 配置、执行控制与 session 事实绑定成 `ResolvedGovernanceSurface`，
//! 供 turn 提交时一次性消费。
//!
//! 装配过程：
//! 1. 从 `ModeCatalog` 查找 mode spec → 调用 compiler 产出治理 compile artifact
//! 2. 构建 `PolicyContext` 和 `AgentCollaborationPolicyContext`
//! 3. 注入 prompt declarations（mode prompt + 协作指导 + skill 声明）
//! 4. 解析 busy policy（是否在 session busy 时分支或拒绝）

use astrcode_core::{
    ResolvedExecutionLimitsSnapshot, ResolvedRuntimeConfig, ResolvedSubagentContextOverrides,
};
use astrcode_governance_contract::ModeId;
use astrcode_prompt_contract::PromptDeclaration;

use super::{
    BuildSurfaceInput, FreshChildGovernanceInput, ResolvedGovernanceSurface,
    ResumedChildGovernanceInput, RootGovernanceInput, SessionGovernanceInput,
};
use crate::{
    AgentSessionPort, ApplicationError, CompiledModeEnvelope, ExecutionControl,
    compile_mode_envelope, compile_mode_envelope_for_child,
    mode_catalog_service::ServerModeCatalog,
};

#[derive(Debug, Clone)]
pub struct GovernanceSurfaceAssembler {
    mode_catalog: ServerModeCatalog,
}

impl GovernanceSurfaceAssembler {
    pub fn new(mode_catalog: ServerModeCatalog) -> Self {
        Self { mode_catalog }
    }

    pub fn runtime_with_control(
        &self,
        runtime: ResolvedRuntimeConfig,
        control: Option<&ExecutionControl>,
        allow_manual_compact: bool,
    ) -> Result<ResolvedRuntimeConfig, ApplicationError> {
        if let Some(control) = control {
            control.validate()?;
            if !allow_manual_compact && control.manual_compact.is_some() {
                return Err(ApplicationError::InvalidArgument(
                    "manualCompact is not valid for prompt submission".to_string(),
                ));
            }
        }
        Ok(runtime)
    }

    fn compile_mode_surface(
        &self,
        mode_id: &ModeId,
        extra_prompt_declarations: Vec<PromptDeclaration>,
    ) -> Result<CompiledModeEnvelope, ApplicationError> {
        let spec = self.mode_catalog.get(mode_id).ok_or_else(|| {
            ApplicationError::InvalidArgument(format!("unknown mode '{}'", mode_id))
        })?;
        compile_mode_envelope(&spec, extra_prompt_declarations).map_err(ApplicationError::from)
    }

    fn compile_child_mode_surface(
        &self,
        mode_id: &ModeId,
    ) -> Result<CompiledModeEnvelope, ApplicationError> {
        let spec = self.mode_catalog.get(mode_id).ok_or_else(|| {
            ApplicationError::InvalidArgument(format!("unknown mode '{}'", mode_id))
        })?;
        compile_mode_envelope_for_child(&spec).map_err(ApplicationError::from)
    }

    fn build_surface(
        &self,
        input: BuildSurfaceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let BuildSurfaceInput {
            compiled,
            runtime,
            resolved_overrides,
            injected_messages,
            leading_prompt_declaration,
            ..
        } = input;
        let mut prompt_declarations: Vec<PromptDeclaration> =
            compiled.envelope.prompt_declarations.clone();
        if let Some(leading) = leading_prompt_declaration {
            prompt_declarations.insert(0, leading);
        }
        prompt_declarations.extend(super::prompt::collaboration_prompt_declarations(
            runtime.agent.max_subrun_depth,
            runtime.agent.max_spawn_per_turn,
        ));
        let surface = ResolvedGovernanceSurface {
            mode_id: compiled.envelope.mode_id.clone(),
            runtime: runtime.clone(),
            prompt_declarations,
            bound_mode_tool_contract: compiled.envelope.bound_tool_contract_snapshot(),
            resolved_limits: ResolvedExecutionLimitsSnapshot,
            resolved_overrides,
            injected_messages,
        };
        Ok(surface)
    }

    pub fn session_surface(
        &self,
        input: SessionGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let runtime = self.runtime_with_control(input.runtime, input.control.as_ref(), false)?;
        let compiled =
            self.compile_mode_surface(&input.mode_id, input.extra_prompt_declarations)?;
        self.build_surface(BuildSurfaceInput {
            compiled,
            runtime,
            resolved_overrides: None,
            injected_messages: Vec::new(),
            leading_prompt_declaration: None,
        })
    }

    pub fn root_surface(
        &self,
        input: RootGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        self.session_surface(SessionGovernanceInput {
            session_id: input.session_id,
            turn_id: input.turn_id,
            working_dir: input.working_dir,
            profile: input.profile,
            mode_id: input.mode_id,
            runtime: input.runtime,
            control: input.control,
            extra_prompt_declarations: Vec::new(),
        })
    }

    pub async fn fresh_child_surface(
        &self,
        session_runtime: &dyn AgentSessionPort,
        input: FreshChildGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let compiled = self.compile_child_mode_surface(&input.mode_id)?;
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
            &ResolvedExecutionLimitsSnapshot,
            compiled.envelope.child_policy.restricted,
        );
        self.build_surface(BuildSurfaceInput {
            compiled,
            runtime: input.runtime,
            resolved_overrides: Some(resolved_overrides),
            injected_messages,
            leading_prompt_declaration: Some(super::build_fresh_child_contract(&delegation)),
        })
    }

    pub fn resumed_child_surface(
        &self,
        input: ResumedChildGovernanceInput,
    ) -> Result<ResolvedGovernanceSurface, ApplicationError> {
        let runtime = input.runtime;
        let compiled = self.compile_mode_surface(&input.mode_id, Vec::new())?;
        let delegation = input.delegation.unwrap_or_else(|| {
            super::build_delegation_metadata(
                "",
                input.message.as_str(),
                &input.resolved_limits,
                false,
            )
        });
        self.build_surface(BuildSurfaceInput {
            compiled,
            runtime,
            resolved_overrides: None,
            injected_messages: Vec::new(),
            leading_prompt_declaration: Some(super::build_resumed_child_contract(
                &delegation,
                input.message.as_str(),
                input.context.as_deref(),
            )),
        })
    }
}

impl Default for GovernanceSurfaceAssembler {
    fn default() -> Self {
        Self::new(
            (*ServerModeCatalog::from_mode_specs(crate::mode::builtin_mode_specs(), Vec::new())
                .expect("builtin governance mode catalog should build"))
            .clone(),
        )
    }
}
