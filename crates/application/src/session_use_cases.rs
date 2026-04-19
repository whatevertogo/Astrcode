/// ! 这是 App 的用例实现，不是 ports
use std::path::Path;

use astrcode_core::{
    AgentEventContext, ChildSessionNode, DeleteProjectResult, ExecutionAccepted, ModeId,
    PromptDeclaration, SessionMeta, StoredEvent,
};

use crate::{
    App, ApplicationError, CompactSessionAccepted, CompactSessionSummary, ExecutionControl,
    ModeSummary, PromptAcceptedSummary, PromptSkillInvocation, SessionControlStateSnapshot,
    SessionListSummary, SessionReplay, SessionTranscriptSnapshot,
    agent::{
        IMPLICIT_ROOT_PROFILE_ID, implicit_session_root_agent_id, root_execution_event_context,
    },
    format_local_rfc3339,
    governance_surface::{GovernanceBusyPolicy, SessionGovernanceInput},
};

impl App {
    pub async fn list_sessions(&self) -> Result<Vec<SessionMeta>, ApplicationError> {
        self.session_runtime
            .list_session_metas()
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<String>,
    ) -> Result<SessionMeta, ApplicationError> {
        let working_dir = working_dir.into();
        self.validate_non_empty("workingDir", &working_dir)?;
        self.session_runtime
            .create_session(working_dir)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        self.session_runtime
            .delete_session(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn fork_session(
        &self,
        session_id: &str,
        fork_point: astrcode_session_runtime::ForkPoint,
    ) -> Result<SessionMeta, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let normalized_session_id = astrcode_session_runtime::normalize_session_id(session_id);
        let result = self
            .session_runtime
            .fork_session(
                &astrcode_core::SessionId::from(normalized_session_id),
                fork_point,
            )
            .await
            .map_err(ApplicationError::from)?;
        let meta = self
            .session_runtime
            .list_session_metas()
            .await
            .map_err(ApplicationError::from)?
            .into_iter()
            .find(|meta| meta.session_id == result.new_session_id.as_str())
            .ok_or_else(|| {
                ApplicationError::Internal(format!(
                    "forked session '{}' was created but metadata is unavailable",
                    result.new_session_id
                ))
            })?;
        Ok(meta)
    }

    pub async fn delete_project(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult, ApplicationError> {
        self.session_runtime
            .delete_project(working_dir)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        self.submit_prompt_with_options(session_id, text, None, None)
            .await
    }

    pub async fn submit_prompt_with_control(
        &self,
        session_id: &str,
        text: String,
        control: Option<ExecutionControl>,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        self.submit_prompt_with_options(session_id, text, control, None)
            .await
    }

    pub async fn submit_prompt_with_options(
        &self,
        session_id: &str,
        text: String,
        control: Option<ExecutionControl>,
        skill_invocation: Option<PromptSkillInvocation>,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        let text = normalize_submission_text(text, skill_invocation.as_ref())?;
        if let Some(control) = &control {
            control.validate()?;
        }
        let working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await?;
        let runtime = self
            .config_service
            .load_resolved_runtime_config(Some(Path::new(&working_dir)))?;
        let root_agent = self.ensure_session_root_agent_context(session_id).await?;
        let prompt_declarations =
            match skill_invocation {
                Some(skill_invocation) => vec![self.build_submission_skill_declaration(
                    Path::new(&working_dir),
                    &skill_invocation,
                )?],
                None => Vec::new(),
            };
        let surface = self.governance_surface.session_surface(
            self.kernel.as_ref(),
            SessionGovernanceInput {
                session_id: session_id.to_string(),
                turn_id: astrcode_core::generate_turn_id(),
                working_dir: working_dir.clone(),
                profile: root_agent
                    .agent_profile
                    .clone()
                    .unwrap_or_else(|| IMPLICIT_ROOT_PROFILE_ID.to_string()),
                mode_id: self
                    .session_runtime
                    .session_mode_state(session_id)
                    .await
                    .map_err(ApplicationError::from)?
                    .current_mode_id,
                runtime,
                control,
                extra_prompt_declarations: prompt_declarations,
                busy_policy: GovernanceBusyPolicy::BranchOnBusy,
            },
        )?;
        self.session_runtime
            .submit_prompt_for_agent(
                session_id,
                text,
                surface.runtime.clone(),
                surface.into_submission(root_agent, None),
            )
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn submit_prompt_summary(
        &self,
        session_id: &str,
        text: String,
        control: Option<ExecutionControl>,
        skill_invocation: Option<PromptSkillInvocation>,
    ) -> Result<PromptAcceptedSummary, ApplicationError> {
        let accepted_control = normalize_prompt_control(control)?;
        let accepted = self
            .submit_prompt_with_options(
                session_id,
                text,
                accepted_control.clone(),
                skill_invocation,
            )
            .await?;
        Ok(PromptAcceptedSummary {
            turn_id: accepted.turn_id.to_string(),
            session_id: accepted.session_id.to_string(),
            branched_from_session_id: accepted.branched_from_session_id,
            accepted_control,
        })
    }

    pub async fn interrupt_session(&self, session_id: &str) -> Result<(), ApplicationError> {
        self.session_runtime
            .interrupt_session(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn compact_session(
        &self,
        session_id: &str,
    ) -> Result<CompactSessionAccepted, ApplicationError> {
        self.compact_session_with_options(session_id, None, None)
            .await
    }

    pub async fn compact_session_with_control(
        &self,
        session_id: &str,
        control: Option<ExecutionControl>,
    ) -> Result<CompactSessionAccepted, ApplicationError> {
        self.compact_session_with_options(session_id, control, None)
            .await
    }

    pub async fn compact_session_with_options(
        &self,
        session_id: &str,
        control: Option<ExecutionControl>,
        instructions: Option<String>,
    ) -> Result<CompactSessionAccepted, ApplicationError> {
        if let Some(control) = &control {
            control.validate()?;
            if control.max_steps.is_some() {
                return Err(ApplicationError::InvalidArgument(
                    "maxSteps is not valid for manual compact".to_string(),
                ));
            }
            if matches!(control.manual_compact, Some(false)) {
                return Err(ApplicationError::InvalidArgument(
                    "manualCompact must be true for manual compact requests".to_string(),
                ));
            }
        }
        let working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await?;
        let runtime = self
            .config_service
            .load_resolved_runtime_config(Some(Path::new(&working_dir)))?;
        let deferred = self
            .session_runtime
            .compact_session(session_id, runtime, instructions)
            .await
            .map_err(ApplicationError::from)?;
        Ok(CompactSessionAccepted { deferred })
    }

    pub async fn compact_session_summary(
        &self,
        session_id: &str,
        control: Option<ExecutionControl>,
        instructions: Option<String>,
    ) -> Result<CompactSessionSummary, ApplicationError> {
        let accepted = self
            .compact_session_with_options(
                session_id,
                normalize_compact_control(control),
                normalize_compact_instructions(instructions),
            )
            .await?;
        Ok(CompactSessionSummary {
            accepted: true,
            deferred: accepted.deferred,
            message: if accepted.deferred {
                "手动 compact 已登记，会在当前 turn 完成后执行。".to_string()
            } else {
                "手动 compact 已执行。".to_string()
            },
        })
    }

    pub async fn session_transcript_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionTranscriptSnapshot, ApplicationError> {
        self.session_runtime
            .session_transcript_snapshot(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn session_control_state(
        &self,
        session_id: &str,
    ) -> Result<SessionControlStateSnapshot, ApplicationError> {
        self.session_runtime
            .session_control_state(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn list_modes(&self) -> Result<Vec<ModeSummary>, ApplicationError> {
        Ok(self.mode_catalog.list())
    }

    pub async fn session_mode_state(
        &self,
        session_id: &str,
    ) -> Result<astrcode_session_runtime::SessionModeSnapshot, ApplicationError> {
        self.session_runtime
            .session_mode_state(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn switch_mode(
        &self,
        session_id: &str,
        target_mode_id: ModeId,
    ) -> Result<astrcode_session_runtime::SessionModeSnapshot, ApplicationError> {
        let current = self
            .session_runtime
            .session_mode_state(session_id)
            .await
            .map_err(ApplicationError::from)?;
        if current.current_mode_id == target_mode_id {
            return Ok(current);
        }
        crate::validate_mode_transition(
            self.mode_catalog.as_ref(),
            &current.current_mode_id,
            &target_mode_id,
        )
        .map_err(ApplicationError::from)?;
        self.session_runtime
            .switch_mode(session_id, current.current_mode_id, target_mode_id)
            .await
            .map_err(ApplicationError::from)?;
        self.session_runtime
            .session_mode_state(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    /// 返回指定 session 的 durable 存储事件。
    ///
    /// Debug Workbench 需要基于服务端真相构造 trace，
    /// 这里显式暴露只读查询入口，避免上层直接穿透到 event store。
    pub async fn session_stored_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<StoredEvent>, ApplicationError> {
        let session_id = astrcode_core::SessionId::from(
            astrcode_session_runtime::normalize_session_id(session_id),
        );
        self.session_runtime
            .session_stored_events(&session_id)
            .await
            .map_err(ApplicationError::from)
    }

    /// 返回指定 session 当前投影出的 child lineage 节点。
    ///
    /// Debug Workbench 的 agent tree 依赖这个稳定投影，不能在前端根据事件流二次猜测。
    pub async fn session_child_nodes(
        &self,
        session_id: &str,
    ) -> Result<Vec<ChildSessionNode>, ApplicationError> {
        self.session_runtime
            .session_child_nodes(session_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<SessionReplay, ApplicationError> {
        self.session_runtime
            .session_replay(session_id, last_event_id)
            .await
            .map_err(ApplicationError::from)
    }

    pub(crate) async fn ensure_session_root_agent_context(
        &self,
        session_id: &str,
    ) -> Result<AgentEventContext, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let normalized_session_id = astrcode_session_runtime::normalize_session_id(session_id);

        if let Some(handle) = self
            .kernel
            .find_root_handle_for_session(&normalized_session_id)
            .await
        {
            return Ok(root_execution_event_context(
                handle.agent_id,
                handle.agent_profile,
            ));
        }

        let handle = self
            .kernel
            .register_root_agent(
                implicit_session_root_agent_id(&normalized_session_id),
                normalized_session_id,
                IMPLICIT_ROOT_PROFILE_ID.to_string(),
            )
            .await
            .map_err(|error| {
                ApplicationError::Internal(format!(
                    "failed to register implicit root agent for session prompt: {error}"
                ))
            })?;
        Ok(root_execution_event_context(
            handle.agent_id,
            handle.agent_profile,
        ))
    }

    fn build_submission_skill_declaration(
        &self,
        working_dir: &Path,
        skill_invocation: &PromptSkillInvocation,
    ) -> Result<PromptDeclaration, ApplicationError> {
        let skill = self
            .composer_skills
            .resolve_skill(working_dir, &skill_invocation.skill_id)
            .ok_or_else(|| {
                ApplicationError::InvalidArgument(format!(
                    "unknown skill slash command: /{}",
                    skill_invocation.skill_id
                ))
            })?;
        Ok(self
            .governance_surface
            .build_submission_skill_declaration(&skill, skill_invocation.user_prompt.as_deref()))
    }
}

pub fn summarize_session_meta(meta: SessionMeta) -> SessionListSummary {
    SessionListSummary {
        session_id: meta.session_id,
        working_dir: meta.working_dir,
        display_name: meta.display_name,
        title: meta.title,
        created_at: format_local_rfc3339(meta.created_at),
        updated_at: format_local_rfc3339(meta.updated_at),
        parent_session_id: meta.parent_session_id,
        parent_storage_seq: meta.parent_storage_seq,
        phase: meta.phase,
    }
}

fn normalize_prompt_control(
    control: Option<ExecutionControl>,
) -> Result<Option<ExecutionControl>, ApplicationError> {
    if let Some(control) = &control {
        control.validate()?;
    }
    Ok(control)
}

fn normalize_submission_text(
    text: String,
    skill_invocation: Option<&PromptSkillInvocation>,
) -> Result<String, ApplicationError> {
    let text = text.trim().to_string();
    let Some(skill_invocation) = skill_invocation else {
        if text.is_empty() {
            return Err(ApplicationError::InvalidArgument(
                "prompt must not be empty".to_string(),
            ));
        }
        return Ok(text);
    };

    let skill_prompt = skill_invocation
        .user_prompt
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if !text.is_empty() && !skill_prompt.is_empty() && text != skill_prompt {
        return Err(ApplicationError::InvalidArgument(
            "skillInvocation.userPrompt must match prompt text".to_string(),
        ));
    }

    if !text.is_empty() {
        Ok(text)
    } else {
        Ok(skill_prompt)
    }
}

fn normalize_compact_control(control: Option<ExecutionControl>) -> Option<ExecutionControl> {
    let mut control = control.unwrap_or(ExecutionControl {
        max_steps: None,
        manual_compact: None,
    });
    if control.manual_compact.is_none() {
        control.manual_compact = Some(true);
    }
    Some(control)
}

fn normalize_compact_instructions(instructions: Option<String>) -> Option<String> {
    instructions
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
