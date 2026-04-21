//! Session 用例（`App` 的 session 相关方法）。
//!
//! 用户直接发起的 session 操作：prompt 提交、compact、mode 切换、
//! session 列表查询、快照查询等。这些方法组装治理面并委托到 session-runtime。

use std::path::Path;

use astrcode_core::{
    AgentEventContext, ChildSessionNode, DeleteProjectResult, ExecutionAccepted, ModeId,
    PromptDeclaration, SessionMeta, StoredEvent,
};
use astrcode_session_runtime::SessionModeSnapshot;

use crate::{
    App, ApplicationError, CompactSessionAccepted, CompactSessionSummary, ExecutionControl,
    ModeSummary, ProjectPlanArchiveDetail, ProjectPlanArchiveSummary, PromptAcceptedSummary,
    PromptSkillInvocation, SessionControlStateSnapshot, SessionListSummary, SessionReplay,
    SessionTranscriptSnapshot,
    agent::{
        IMPLICIT_ROOT_PROFILE_ID, implicit_session_root_agent_id, root_execution_event_context,
    },
    format_local_rfc3339,
    governance_surface::{GovernanceBusyPolicy, SessionGovernanceInput},
    session_plan::{
        active_plan_requires_approval, advance_plan_workflow_to_execution,
        bootstrap_plan_workflow_state, build_execute_phase_prompt_declaration,
        build_plan_exit_declaration, build_plan_prompt_context, build_plan_prompt_declarations,
        copy_session_plan_artifacts, current_mode_requires_plan_context,
        list_project_plan_archives, load_session_plan_state, mark_active_session_plan_approved,
        parse_plan_approval, parse_plan_workflow_signal, planning_phase_allows_review_mode,
        read_project_plan_archive, revert_execution_to_planning_workflow_state,
    },
    workflow::{
        EXECUTING_PHASE_ID, PLANNING_PHASE_ID, WorkflowInstanceState, WorkflowStateService,
    },
};

#[derive(Debug, Default)]
struct PreparedSessionSubmission {
    current_mode_id: ModeId,
    prompt_declarations: Vec<PromptDeclaration>,
}

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
        let source_working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await?;
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
        copy_session_plan_artifacts(
            session_id,
            result.new_session_id.as_str(),
            Path::new(&source_working_dir),
        )?;
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

    pub fn list_project_plan_archives(
        &self,
        working_dir: &Path,
    ) -> Result<Vec<ProjectPlanArchiveSummary>, ApplicationError> {
        list_project_plan_archives(working_dir)
    }

    pub fn read_project_plan_archive(
        &self,
        working_dir: &Path,
        archive_id: &str,
    ) -> Result<Option<ProjectPlanArchiveDetail>, ApplicationError> {
        self.validate_non_empty("archiveId", archive_id)?;
        read_project_plan_archive(working_dir, archive_id)
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

    /// 带 skill 调用选项的 prompt 提交。
    ///
    /// 完整流程：
    /// 1. 规范化文本（处理 skill invocation 与纯文本的交互）
    /// 2. 校验 ExecutionControl 参数
    /// 3. 加载 runtime 配置 + 确保 session root agent context
    /// 4. 若有 skill invocation，解析 skill 并构建 prompt declaration
    /// 5. 构建治理面（工具白名单、审批策略、协作指导等）
    /// 6. 委托 session-runtime 提交 prompt
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
        let mut current_mode_id = self
            .session_runtime
            .session_mode_state(session_id)
            .await
            .map_err(ApplicationError::from)?
            .current_mode_id;
        let submission = self
            .prepare_session_submission(
                session_id,
                Path::new(&working_dir),
                &text,
                current_mode_id.clone(),
            )
            .await?;
        current_mode_id = submission.current_mode_id;
        let mut prompt_declarations = submission.prompt_declarations;

        if let Some(skill_invocation) = skill_invocation {
            prompt_declarations.push(
                self.build_submission_skill_declaration(
                    Path::new(&working_dir),
                    &skill_invocation,
                )?,
            );
        }
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
                mode_id: current_mode_id,
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

    async fn prepare_session_submission(
        &self,
        session_id: &str,
        working_dir: &Path,
        text: &str,
        current_mode_id: ModeId,
    ) -> Result<PreparedSessionSubmission, ApplicationError> {
        let workflow_state_path = WorkflowStateService::state_path(session_id, working_dir)?;
        let workflow_state_exists = workflow_state_path.exists();
        let mut workflow_state = self
            .workflow()
            .load_active_workflow(session_id, working_dir)?;
        if workflow_state.is_none() && !workflow_state_exists {
            workflow_state =
                bootstrap_plan_workflow_state(session_id, working_dir, &current_mode_id)?;
            if let Some(state) = workflow_state.as_ref() {
                self.workflow()
                    .persist_active_workflow(session_id, working_dir, state)?;
            }
        }

        match workflow_state {
            Some(workflow_state) => {
                self.prepare_active_workflow_submission(
                    session_id,
                    working_dir,
                    text,
                    current_mode_id,
                    workflow_state,
                )
                .await
            },
            None => {
                self.prepare_mode_only_submission(session_id, working_dir, text, current_mode_id)
                    .await
            },
        }
    }

    async fn prepare_mode_only_submission(
        &self,
        session_id: &str,
        working_dir: &Path,
        text: &str,
        mut current_mode_id: ModeId,
    ) -> Result<PreparedSessionSubmission, ApplicationError> {
        let mut prompt_declarations = Vec::new();
        let plan_state = load_session_plan_state(session_id, working_dir)?;
        let plan_approval = parse_plan_approval(text);

        if active_plan_requires_approval(plan_state.as_ref()) && plan_approval.approved {
            let approved_plan = mark_active_session_plan_approved(session_id, working_dir)?;
            if current_mode_id == ModeId::plan() {
                self.switch_mode(session_id, ModeId::code()).await?;
                current_mode_id = ModeId::code();
            }
            if let Some(summary) = approved_plan {
                prompt_declarations.push(build_plan_exit_declaration(session_id, &summary));
            }
        } else if current_mode_id == ModeId::plan()
            && current_mode_requires_plan_context(&current_mode_id)
            && !plan_approval.approved
        {
            let context = build_plan_prompt_context(session_id, working_dir, text)?;
            prompt_declarations.extend(build_plan_prompt_declarations(session_id, &context));
        }

        Ok(PreparedSessionSubmission {
            current_mode_id,
            prompt_declarations,
        })
    }

    async fn prepare_active_workflow_submission(
        &self,
        session_id: &str,
        working_dir: &Path,
        text: &str,
        mut current_mode_id: ModeId,
        mut workflow_state: WorkflowInstanceState,
    ) -> Result<PreparedSessionSubmission, ApplicationError> {
        let plan_state = load_session_plan_state(session_id, working_dir)?;
        let signal = parse_plan_workflow_signal(text, plan_state.as_ref());
        let mut prompt_declarations = Vec::new();

        if let Some(signal) = signal {
            if let Some(transition) = self
                .workflow()
                .transition_for_signal(&workflow_state, signal)?
            {
                workflow_state = match (
                    transition.source_phase_id.as_str(),
                    transition.target_phase_id.as_str(),
                ) {
                    (PLANNING_PHASE_ID, EXECUTING_PHASE_ID) => {
                        advance_plan_workflow_to_execution(session_id, working_dir)?
                            .map(|(state, declaration)| {
                                prompt_declarations.push(declaration);
                                state
                            })
                            .ok_or_else(|| {
                                ApplicationError::Internal(
                                    "plan approval signal did not produce an executing workflow \
                                     state"
                                        .to_string(),
                                )
                            })?
                    },
                    (EXECUTING_PHASE_ID, PLANNING_PHASE_ID) => {
                        revert_execution_to_planning_workflow_state(session_id, working_dir)?
                    },
                    _ => {
                        return Err(ApplicationError::Internal(format!(
                            "unsupported workflow transition '{} -> {}'",
                            transition.source_phase_id, transition.target_phase_id
                        )));
                    },
                };
                self.workflow().persist_active_workflow(
                    session_id,
                    working_dir,
                    &workflow_state,
                )?;
            }
        }

        current_mode_id = self
            .reconcile_workflow_phase_mode(
                session_id,
                working_dir,
                current_mode_id,
                &workflow_state,
                plan_state.as_ref(),
            )
            .await?;

        match workflow_state.current_phase_id.as_str() {
            PLANNING_PHASE_ID => {
                let context = build_plan_prompt_context(session_id, working_dir, text)?;
                prompt_declarations.extend(build_plan_prompt_declarations(session_id, &context));
            },
            EXECUTING_PHASE_ID => {
                if prompt_declarations.is_empty() {
                    if let Some(declaration) =
                        build_execute_phase_prompt_declaration(session_id, &workflow_state)?
                    {
                        prompt_declarations.push(declaration);
                    }
                }
            },
            other => {
                return Err(ApplicationError::Internal(format!(
                    "unsupported workflow phase '{other}'"
                )));
            },
        }

        Ok(PreparedSessionSubmission {
            current_mode_id,
            prompt_declarations,
        })
    }

    async fn reconcile_workflow_phase_mode(
        &self,
        session_id: &str,
        working_dir: &Path,
        current_mode_id: ModeId,
        workflow_state: &WorkflowInstanceState,
        plan_state: Option<&astrcode_core::SessionPlanState>,
    ) -> Result<ModeId, ApplicationError> {
        let phase = self.workflow().phase(workflow_state)?;
        if phase.mode_id == current_mode_id {
            return Ok(current_mode_id);
        }
        if workflow_state.current_phase_id == PLANNING_PHASE_ID
            && planning_phase_allows_review_mode(&current_mode_id, plan_state)
        {
            return Ok(current_mode_id);
        }

        match self.switch_mode(session_id, phase.mode_id.clone()).await {
            Ok(SessionModeSnapshot {
                current_mode_id, ..
            }) => Ok(current_mode_id),
            Err(error) => {
                let state_path = WorkflowStateService::state_path(session_id, working_dir)?;
                log::warn!(
                    "workflow phase '{}' persisted in '{}' but mode reconcile to '{}' failed: {}",
                    workflow_state.current_phase_id,
                    state_path.display(),
                    phase.mode_id,
                    error
                );
                Err(error)
            },
        }
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

    /// 确保 session 存在一个 root agent context，如果没有则自动注册隐式 root agent。
    ///
    /// 查找逻辑：先通过 kernel 查找已有 handle，找不到则注册隐式 root agent
    /// （ID 为 `root-agent:{session_id}`，profile 为 `default`）。
    /// 这是 prompt 提交前的前置步骤，保证 session 总有一个可用的 agent context。
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

/// 规范化 prompt 提交文本，处理 skill invocation 与纯文本的交互。
///
/// - 纯文本提交：不允许空文本
/// - Skill invocation：文本可以为空（由 skill prompt 填充）， 但如果同时提供了文本和 skill
///   userPrompt，两者必须一致
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

/// 为手动 compact 请求构建 ExecutionControl。
///
/// 强制设置 `manual_compact = true`（如果调用方未指定），
/// 因为 compact 的语义要求这个标志。
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Arc,
    };

    use astrcode_core::{
        ExecutionTaskItem, ExecutionTaskStatus, ModeId, SessionPlanState, SessionPlanStatus,
        TaskSnapshot,
    };
    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::{
        App, AppKernelPort, AppSessionPort, ComposerResolvedSkill, ComposerSkillPort,
        McpConfigScope, McpPort, McpServerStatusView, McpService,
        agent::test_support::{AgentTestHarness, TestLlmBehavior, build_agent_test_harness},
        composer::ComposerSkillSummary,
        governance_surface::GovernanceSurfaceAssembler,
        mcp::RegisterMcpServerInput,
        mode::builtin_mode_catalog,
        session_plan::session_plan_dir,
        test_support::StubSessionPort,
    };

    struct EmptyComposerSkillPort;

    impl ComposerSkillPort for EmptyComposerSkillPort {
        fn list_skill_summaries(&self, _working_dir: &Path) -> Vec<ComposerSkillSummary> {
            Vec::new()
        }

        fn resolve_skill(
            &self,
            _working_dir: &Path,
            _skill_id: &str,
        ) -> Option<ComposerResolvedSkill> {
            None
        }
    }

    struct NoopMcpPort;

    #[async_trait]
    impl McpPort for NoopMcpPort {
        async fn list_server_status(&self) -> Vec<McpServerStatusView> {
            Vec::new()
        }

        async fn approve_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn reject_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn reconnect_server(&self, _name: &str) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn reset_project_choices(&self) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn upsert_server(
            &self,
            _input: &RegisterMcpServerInput,
        ) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn remove_server(
            &self,
            _scope: McpConfigScope,
            _name: &str,
        ) -> Result<(), ApplicationError> {
            Ok(())
        }

        async fn set_server_enabled(
            &self,
            _scope: McpConfigScope,
            _name: &str,
            _enabled: bool,
        ) -> Result<(), ApplicationError> {
            Ok(())
        }
    }

    struct SessionUseCasesHarness {
        _agent_harness: AgentTestHarness,
        _workspace_root: tempfile::TempDir,
        app: App,
        session_port: Arc<StubSessionPort>,
        session_id: String,
        working_dir: PathBuf,
    }

    impl SessionUseCasesHarness {
        fn new(initial_mode: ModeId) -> Self {
            let agent_harness = build_agent_test_harness(TestLlmBehavior::Succeed {
                content: "ok".to_string(),
            })
            .expect("agent harness should build");
            let workspace_root = tempfile::tempdir().expect("workspace root should exist");
            let working_dir = workspace_root.path().join("workspace");
            fs::create_dir_all(&working_dir).expect("workspace should exist");
            let session_port = Arc::new(StubSessionPort {
                working_dir: Some(working_dir.display().to_string()),
                mode_state: Arc::new(std::sync::Mutex::new(Some(
                    astrcode_session_runtime::SessionModeSnapshot {
                        current_mode_id: initial_mode,
                        last_mode_changed_at: None,
                    },
                ))),
                ..StubSessionPort::default()
            });
            let kernel: Arc<dyn AppKernelPort> = agent_harness.kernel.clone();
            let session_runtime: Arc<dyn AppSessionPort> = session_port.clone();
            let app = App::new(
                kernel,
                session_runtime,
                agent_harness.profiles.clone(),
                agent_harness.config_service.clone(),
                Arc::new(EmptyComposerSkillPort),
                Arc::new(GovernanceSurfaceAssembler::default()),
                Arc::new(builtin_mode_catalog().expect("mode catalog should build")),
                Arc::new(McpService::new(Arc::new(NoopMcpPort))),
                Arc::new(agent_harness.service.clone()),
            );
            Self {
                _agent_harness: agent_harness,
                _workspace_root: workspace_root,
                app,
                session_port,
                session_id: "session-a".to_string(),
                working_dir,
            }
        }

        fn write_plan_state(
            &self,
            status: SessionPlanStatus,
            content: &str,
        ) -> Result<(), ApplicationError> {
            let plan_dir = session_plan_dir(&self.session_id, &self.working_dir)?;
            fs::create_dir_all(&plan_dir).expect("plan dir should exist");
            fs::write(plan_dir.join("plan.md"), content).expect("plan content should be written");
            let now = Utc::now();
            let state = SessionPlanState {
                active_plan_slug: "plan".to_string(),
                title: "Plan".to_string(),
                status,
                created_at: now,
                updated_at: now,
                reviewed_plan_digest: None,
                approved_at: None,
                archived_plan_digest: None,
                archived_at: None,
            };
            fs::write(
                plan_dir.join("state.json"),
                serde_json::to_string_pretty(&state).expect("plan state should serialize"),
            )
            .expect("plan state should be written");
            Ok(())
        }
    }

    #[tokio::test]
    async fn corrupted_workflow_state_downgrades_to_mode_only_submission() {
        let harness = SessionUseCasesHarness::new(ModeId::plan());
        harness
            .write_plan_state(
                SessionPlanStatus::AwaitingApproval,
                "# Plan\n\n## Implementation Steps\n- Keep refining\n",
            )
            .expect("plan state should be seeded");
        let workflow_path =
            WorkflowStateService::state_path(&harness.session_id, &harness.working_dir)
                .expect("workflow path should resolve");
        fs::create_dir_all(
            workflow_path
                .parent()
                .expect("workflow parent should exist"),
        )
        .expect("workflow parent should exist");
        fs::write(&workflow_path, "{not-json").expect("invalid workflow should be written");

        harness
            .app
            .submit_prompt(&harness.session_id, "继续完善计划".to_string())
            .await
            .expect("submission should degrade to mode-only path");

        let submissions = harness
            .session_port
            .recorded_submissions
            .lock()
            .expect("submission record lock should work")
            .clone();
        assert_eq!(submissions.len(), 1);
        assert!(
            submissions[0]
                .prompt_declarations
                .iter()
                .any(|declaration| declaration.origin.as_deref() == Some("session-plan:facts"))
        );
        assert!(
            !submissions[0]
                .prompt_declarations
                .iter()
                .any(|declaration| declaration.origin.as_deref()
                    == Some("session-plan:execute-bridge"))
        );
    }

    #[tokio::test]
    async fn semantically_invalid_workflow_state_downgrades_to_mode_only_submission() {
        let harness = SessionUseCasesHarness::new(ModeId::code());
        harness
            .write_plan_state(
                SessionPlanStatus::Approved,
                "# Plan\n\n## Implementation Steps\n- Keep executing through mode-only fallback\n",
            )
            .expect("plan state should be seeded");
        let workflow_path =
            WorkflowStateService::state_path(&harness.session_id, &harness.working_dir)
                .expect("workflow path should resolve");
        fs::create_dir_all(
            workflow_path
                .parent()
                .expect("workflow parent should exist"),
        )
        .expect("workflow parent should exist");
        fs::write(
            &workflow_path,
            serde_json::json!({
                "workflowId": "plan_execute",
                "currentPhaseId": EXECUTING_PHASE_ID,
                "artifactRefs": {
                    "canonical-plan": {
                        "artifactKind": "canonical-plan",
                        "path": harness
                            .working_dir
                            .join("sessions")
                            .join(&harness.session_id)
                            .join("plan")
                            .join("plan.md")
                            .display()
                            .to_string()
                    }
                },
                "bridgeState": {
                    "bridgeKind": "noop",
                    "sourcePhaseId": PLANNING_PHASE_ID,
                    "targetPhaseId": EXECUTING_PHASE_ID,
                    "schemaVersion": 1,
                    "payload": {}
                },
                "updatedAt": Utc::now().to_rfc3339()
            })
            .to_string(),
        )
        .expect("invalid semantic workflow should be written");

        harness
            .app
            .submit_prompt(&harness.session_id, "开始实现".to_string())
            .await
            .expect("submission should degrade to mode-only path");

        let submissions = harness
            .session_port
            .recorded_submissions
            .lock()
            .expect("submission record lock should work")
            .clone();
        assert_eq!(submissions.len(), 1);
        assert!(
            !submissions[0]
                .prompt_declarations
                .iter()
                .any(|declaration| declaration.origin.as_deref()
                    == Some("session-plan:execute-bridge"))
        );
    }

    #[tokio::test]
    async fn approval_persists_executing_phase_before_mode_switch_and_reconciles_later() {
        let harness = SessionUseCasesHarness::new(ModeId::plan());
        harness
            .write_plan_state(
                SessionPlanStatus::AwaitingApproval,
                "# Plan\n\n## Implementation Steps\n1. Implement workflow orchestration\n2. Add \
                 tests\n",
            )
            .expect("plan state should be seeded");
        let workflow_state = bootstrap_plan_workflow_state(
            &harness.session_id,
            &harness.working_dir,
            &ModeId::plan(),
        )
        .expect("bootstrap should succeed")
        .expect("planning workflow should bootstrap");
        WorkflowStateService::persist(&harness.session_id, &harness.working_dir, &workflow_state)
            .expect("workflow state should persist");
        let existing_snapshot = TaskSnapshot {
            owner: astrcode_session_runtime::ROOT_AGENT_ID.to_string(),
            items: vec![ExecutionTaskItem {
                content: "保持现有 task snapshot".to_string(),
                status: ExecutionTaskStatus::InProgress,
                active_form: Some("正在保持现有 task snapshot".to_string()),
            }],
        };
        *harness
            .session_port
            .active_task_snapshot
            .lock()
            .expect("active task snapshot lock should work") = Some(existing_snapshot.clone());
        *harness
            .session_port
            .switch_mode_error
            .lock()
            .expect("mode switch error lock should work") =
            Some("forced mode switch failure".to_string());

        let error = harness
            .app
            .submit_prompt(&harness.session_id, "同意".to_string())
            .await
            .expect_err("mode reconcile failure should surface");
        assert!(
            error.to_string().contains("forced mode switch failure"),
            "unexpected error: {error}"
        );

        let persisted = WorkflowStateService::load(&harness.session_id, &harness.working_dir)
            .expect("workflow state should load")
            .expect("workflow state should exist");
        assert_eq!(persisted.current_phase_id, EXECUTING_PHASE_ID);

        *harness
            .session_port
            .switch_mode_error
            .lock()
            .expect("mode switch error lock should work") = None;

        harness
            .app
            .submit_prompt(&harness.session_id, "开始实现".to_string())
            .await
            .expect("second submission should reconcile mode and proceed");

        let submissions = harness
            .session_port
            .recorded_submissions
            .lock()
            .expect("submission record lock should work")
            .clone();
        assert_eq!(submissions.len(), 1);
        assert!(
            submissions[0]
                .prompt_declarations
                .iter()
                .any(|declaration| declaration.origin.as_deref()
                    == Some("session-plan:execute-bridge"))
        );
        let mode_switches = harness
            .session_port
            .recorded_mode_switches
            .lock()
            .expect("mode switch record lock should work")
            .clone();
        assert_eq!(mode_switches.len(), 1);
        assert_eq!(mode_switches[0].to, ModeId::code());
        assert_eq!(
            harness
                .session_port
                .active_task_snapshot
                .lock()
                .expect("active task snapshot lock should work")
                .clone(),
            Some(existing_snapshot)
        );
    }

    #[tokio::test]
    async fn executing_replan_signal_returns_to_planning_overlay() {
        let harness = SessionUseCasesHarness::new(ModeId::code());
        harness
            .write_plan_state(
                SessionPlanStatus::Approved,
                "# Plan\n\n## Implementation Steps\n- Keep the plan artifact stable\n",
            )
            .expect("plan state should be seeded");
        let workflow_state = bootstrap_plan_workflow_state(
            &harness.session_id,
            &harness.working_dir,
            &ModeId::code(),
        )
        .expect("bootstrap should succeed")
        .expect("executing workflow should bootstrap");
        WorkflowStateService::persist(&harness.session_id, &harness.working_dir, &workflow_state)
            .expect("workflow state should persist");
        let existing_snapshot = TaskSnapshot {
            owner: astrcode_session_runtime::ROOT_AGENT_ID.to_string(),
            items: vec![ExecutionTaskItem {
                content: "保留执行 task snapshot".to_string(),
                status: ExecutionTaskStatus::InProgress,
                active_form: Some("正在保留执行 task snapshot".to_string()),
            }],
        };
        *harness
            .session_port
            .active_task_snapshot
            .lock()
            .expect("active task snapshot lock should work") = Some(existing_snapshot.clone());

        harness
            .app
            .submit_prompt(&harness.session_id, "重新计划".to_string())
            .await
            .expect("replan should transition back to planning");

        let persisted = WorkflowStateService::load(&harness.session_id, &harness.working_dir)
            .expect("workflow state should load")
            .expect("workflow state should exist");
        assert_eq!(persisted.current_phase_id, PLANNING_PHASE_ID);
        let submissions = harness
            .session_port
            .recorded_submissions
            .lock()
            .expect("submission record lock should work")
            .clone();
        assert_eq!(submissions.len(), 1);
        assert!(
            submissions[0]
                .prompt_declarations
                .iter()
                .any(|declaration| declaration.origin.as_deref() == Some("session-plan:facts"))
        );
        assert!(
            !submissions[0]
                .prompt_declarations
                .iter()
                .any(|declaration| declaration.origin.as_deref()
                    == Some("session-plan:execute-bridge"))
        );
        let mode_switches = harness
            .session_port
            .recorded_mode_switches
            .lock()
            .expect("mode switch record lock should work")
            .clone();
        assert_eq!(mode_switches.len(), 1);
        assert_eq!(mode_switches[0].to, ModeId::plan());
        assert_eq!(
            harness
                .session_port
                .active_task_snapshot
                .lock()
                .expect("active task snapshot lock should work")
                .clone(),
            Some(existing_snapshot)
        );
    }
}
