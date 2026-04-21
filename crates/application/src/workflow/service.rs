use std::{collections::BTreeMap, fs, future::Future, path::Path};

use astrcode_core::{
    ModeId, PromptDeclaration, SessionPlanState, SessionPlanStatus, session_plan_content_digest,
};
use chrono::Utc;

use crate::{
    ApplicationError,
    session_plan::{
        active_plan_requires_approval, build_execute_bridge_declaration, load_session_plan_state,
        mark_active_session_plan_approved, planning_phase_allows_review_mode,
        session_plan_markdown_path,
    },
    workflow::{
        EXECUTING_PHASE_ID, PLAN_EXECUTE_WORKFLOW_ID, PLANNING_PHASE_ID, PlanImplementationStep,
        PlanToExecuteBridgeState, WorkflowArtifactRef, WorkflowInstanceState, WorkflowOrchestrator,
    },
};

/// 基于当前 mode / plan 状态推导初始 workflow state。
pub(crate) fn bootstrap_plan_workflow_state(
    session_id: &str,
    working_dir: &Path,
    current_mode_id: &astrcode_core::ModeId,
) -> Result<Option<WorkflowInstanceState>, ApplicationError> {
    let plan_state = load_session_plan_state(session_id, working_dir)?;
    if current_mode_id == &astrcode_core::ModeId::plan()
        || active_plan_requires_approval(plan_state.as_ref())
    {
        return Ok(Some(build_planning_workflow_state(
            session_id,
            working_dir,
            plan_state.as_ref(),
        )?));
    }
    if plan_state
        .as_ref()
        .is_some_and(|state| state.status == SessionPlanStatus::Approved)
    {
        return Ok(Some(build_executing_workflow_state(
            session_id,
            working_dir,
            plan_state
                .as_ref()
                .expect("approved plan state should exist"),
        )?));
    }
    Ok(None)
}

/// 执行 planning -> executing 迁移，并生成 execute bridge prompt。
pub(crate) fn advance_plan_workflow_to_execution(
    session_id: &str,
    working_dir: &Path,
) -> Result<Option<(WorkflowInstanceState, PromptDeclaration)>, ApplicationError> {
    let approved_plan = mark_active_session_plan_approved(session_id, working_dir)?;
    let Some(plan_state) = load_session_plan_state(session_id, working_dir)? else {
        return Ok(None);
    };
    if plan_state.status != SessionPlanStatus::Approved {
        return Ok(None);
    }

    let next_state = build_executing_workflow_state(session_id, working_dir, &plan_state)?;
    let bridge = next_state
        .bridge_state
        .as_ref()
        .ok_or_else(|| {
            ApplicationError::Internal(
                "executing workflow state must include plan bridge state".to_string(),
            )
        })
        .and_then(PlanToExecuteBridgeState::from_bridge_state)?;
    let mut declaration = build_execute_bridge_declaration(session_id, &bridge);
    if let Some(summary) = approved_plan {
        declaration.content.push_str(&format!(
            "\n- approvedPlanSlug: {}\n- approvedPlanStatus: {}",
            summary.slug, summary.status
        ));
    }
    Ok(Some((next_state, declaration)))
}

pub(crate) fn revert_execution_to_planning_workflow_state(
    session_id: &str,
    working_dir: &Path,
) -> Result<WorkflowInstanceState, ApplicationError> {
    let plan_state = load_session_plan_state(session_id, working_dir)?;
    build_planning_workflow_state(session_id, working_dir, plan_state.as_ref())
}

pub(crate) fn build_execute_phase_prompt_declaration(
    session_id: &str,
    workflow_state: &WorkflowInstanceState,
) -> Result<Option<PromptDeclaration>, ApplicationError> {
    let Some(bridge_state) = workflow_state.bridge_state.as_ref() else {
        return Ok(None);
    };
    let bridge = PlanToExecuteBridgeState::from_bridge_state(bridge_state)?;
    Ok(Some(build_execute_bridge_declaration(session_id, &bridge)))
}

pub(crate) async fn reconcile_workflow_phase_mode<F, Fut>(
    orchestrator: &WorkflowOrchestrator,
    session_id: &str,
    working_dir: &Path,
    current_mode_id: ModeId,
    workflow_state: &WorkflowInstanceState,
    plan_state: Option<&SessionPlanState>,
    mut switch_mode: F,
) -> Result<ModeId, ApplicationError>
where
    F: FnMut(ModeId) -> Fut,
    Fut: Future<Output = Result<astrcode_session_runtime::SessionModeSnapshot, ApplicationError>>,
{
    let phase = orchestrator.phase(workflow_state)?;
    if phase.mode_id == current_mode_id {
        return Ok(current_mode_id);
    }
    if workflow_state.current_phase_id == PLANNING_PHASE_ID
        && planning_phase_allows_review_mode(&current_mode_id, plan_state)
    {
        return Ok(current_mode_id);
    }

    match switch_mode(phase.mode_id.clone()).await {
        Ok(astrcode_session_runtime::SessionModeSnapshot {
            current_mode_id, ..
        }) => Ok(current_mode_id),
        Err(error) => {
            let state_path =
                crate::workflow::WorkflowStateService::state_path(session_id, working_dir)?;
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

fn build_planning_workflow_state(
    session_id: &str,
    working_dir: &Path,
    plan_state: Option<&SessionPlanState>,
) -> Result<WorkflowInstanceState, ApplicationError> {
    let mut artifact_refs = BTreeMap::new();
    if let Some(plan_state) = plan_state {
        if let Some(plan_artifact) = current_plan_artifact_ref(session_id, working_dir, plan_state)?
        {
            artifact_refs.insert("canonical-plan".to_string(), plan_artifact);
        }
    }
    Ok(WorkflowInstanceState {
        workflow_id: PLAN_EXECUTE_WORKFLOW_ID.to_string(),
        current_phase_id: PLANNING_PHASE_ID.to_string(),
        artifact_refs,
        bridge_state: None,
        updated_at: plan_state
            .map(|state| state.updated_at)
            .unwrap_or_else(Utc::now),
    })
}

fn build_executing_workflow_state(
    session_id: &str,
    working_dir: &Path,
    plan_state: &SessionPlanState,
) -> Result<WorkflowInstanceState, ApplicationError> {
    let bridge = load_plan_to_execute_bridge_state(session_id, working_dir, plan_state)?;
    let plan_artifact = bridge.plan_artifact.clone();
    let bridge_state = bridge.into_bridge_state(PLANNING_PHASE_ID, EXECUTING_PHASE_ID)?;
    Ok(WorkflowInstanceState {
        workflow_id: PLAN_EXECUTE_WORKFLOW_ID.to_string(),
        current_phase_id: EXECUTING_PHASE_ID.to_string(),
        artifact_refs: BTreeMap::from([("canonical-plan".to_string(), plan_artifact)]),
        bridge_state: Some(bridge_state),
        updated_at: plan_state.updated_at,
    })
}

fn current_plan_artifact_ref(
    session_id: &str,
    working_dir: &Path,
    plan_state: &SessionPlanState,
) -> Result<Option<WorkflowArtifactRef>, ApplicationError> {
    let plan_path =
        session_plan_markdown_path(session_id, working_dir, &plan_state.active_plan_slug)?;
    let Ok(content) = fs::read_to_string(&plan_path) else {
        return Ok(None);
    };
    Ok(Some(WorkflowArtifactRef {
        artifact_kind: "canonical-plan".to_string(),
        path: plan_path.display().to_string(),
        content_digest: Some(session_plan_content_digest(content.trim())),
    }))
}

fn load_plan_to_execute_bridge_state(
    session_id: &str,
    working_dir: &Path,
    plan_state: &SessionPlanState,
) -> Result<PlanToExecuteBridgeState, ApplicationError> {
    let (plan_artifact, plan_content) =
        load_required_plan_artifact(session_id, working_dir, plan_state)?;
    Ok(PlanToExecuteBridgeState {
        plan_artifact,
        plan_title: plan_state.title.clone(),
        implementation_steps: extract_implementation_steps(&plan_content),
        approved_at: plan_state.approved_at,
    })
}

fn load_required_plan_artifact(
    session_id: &str,
    working_dir: &Path,
    plan_state: &SessionPlanState,
) -> Result<(WorkflowArtifactRef, String), ApplicationError> {
    let plan_path =
        session_plan_markdown_path(session_id, working_dir, &plan_state.active_plan_slug)?;
    let plan_content = match fs::read_to_string(&plan_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApplicationError::Internal(format!(
                "approved plan artifact '{}' is missing",
                plan_path.display()
            )));
        },
        Err(error) => return Err(io_error("reading", &plan_path, error)),
    };
    Ok((
        WorkflowArtifactRef {
            artifact_kind: "canonical-plan".to_string(),
            path: plan_path.display().to_string(),
            content_digest: Some(session_plan_content_digest(plan_content.trim())),
        },
        plan_content,
    ))
}

fn extract_implementation_steps(content: &str) -> Vec<PlanImplementationStep> {
    let mut in_steps_section = false;
    let mut steps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            if in_steps_section {
                break;
            }
            in_steps_section = matches!(
                trimmed,
                "## Implementation Steps" | "## 实现步骤" | "## 实施步骤"
            );
            continue;
        }
        if !in_steps_section {
            continue;
        }

        let parsed_step = trimmed
            .strip_prefix("- ")
            .map(|summary| (None, summary))
            .or_else(|| trimmed.strip_prefix("* ").map(|summary| (None, summary)))
            .or_else(|| trimmed.strip_prefix("+ ").map(|summary| (None, summary)))
            .or_else(|| {
                trimmed.split_once(". ").and_then(|(prefix, rest)| {
                    prefix
                        .parse::<usize>()
                        .ok()
                        .map(|parsed_index| (Some(parsed_index), rest))
                })
            })
            .map(|(parsed_index, summary)| (parsed_index, summary.trim()))
            .filter(|(_, summary)| !summary.is_empty());
        let Some((parsed_index, summary)) = parsed_step else {
            continue;
        };

        let summary = summary.to_string();
        steps.push(PlanImplementationStep {
            index: parsed_index.unwrap_or(steps.len() + 1),
            title: summary.clone(),
            summary,
        });
    }

    steps
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ApplicationError {
    ApplicationError::Internal(format!("{action} '{}' failed: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use astrcode_core::{ModeId, SessionPlanState, SessionPlanStatus, WorkflowInstanceState};
    use astrcode_session_runtime::SessionModeSnapshot;
    use chrono::{TimeZone, Utc};

    use super::{
        advance_plan_workflow_to_execution, bootstrap_plan_workflow_state,
        extract_implementation_steps, reconcile_workflow_phase_mode,
        revert_execution_to_planning_workflow_state,
    };
    use crate::{
        ApplicationError,
        workflow::{
            EXECUTING_PHASE_ID, PLAN_EXECUTE_WORKFLOW_ID, PLANNING_PHASE_ID, WorkflowOrchestrator,
        },
    };

    fn prepare_working_dir() -> (astrcode_core::test_support::TestEnvGuard, PathBuf) {
        let guard = astrcode_core::test_support::TestEnvGuard::new();
        let working_dir = guard.home_dir().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should exist");
        (guard, working_dir)
    }

    fn sample_plan_state(status: SessionPlanStatus) -> SessionPlanState {
        let now = Utc
            .with_ymd_and_hms(2026, 4, 21, 9, 0, 0)
            .single()
            .expect("datetime should be valid");
        SessionPlanState {
            active_plan_slug: "cleanup-crates".to_string(),
            title: "Cleanup crates".to_string(),
            status,
            created_at: now,
            updated_at: now,
            reviewed_plan_digest: None,
            approved_at: None,
            archived_plan_digest: None,
            archived_at: None,
        }
    }

    fn persist_plan_fixture(
        session_id: &str,
        working_dir: &Path,
        status: SessionPlanStatus,
        write_markdown: bool,
    ) -> SessionPlanState {
        let mut state = sample_plan_state(status.clone());
        if matches!(status, SessionPlanStatus::Approved) {
            state.approved_at = Some(state.updated_at);
        }
        let plan_dir = crate::session_plan::session_plan_dir(session_id, working_dir)
            .expect("plan dir should resolve");
        fs::create_dir_all(&plan_dir).expect("plan dir should exist");
        fs::write(
            plan_dir.join("state.json"),
            serde_json::to_string_pretty(&state).expect("plan state should serialize"),
        )
        .expect("plan state should persist");
        if write_markdown {
            let plan_path = crate::session_plan::session_plan_markdown_path(
                session_id,
                working_dir,
                &state.active_plan_slug,
            )
            .expect("plan path should resolve");
            fs::write(
                plan_path,
                "# Plan: Cleanup crates\n\n## Implementation Steps\n1. Audit crate boundaries\n- \
                 Remove duplicated workflow state\n",
            )
            .expect("plan markdown should persist");
        }
        state
    }

    fn workflow_state(current_phase_id: &str) -> WorkflowInstanceState {
        WorkflowInstanceState {
            workflow_id: PLAN_EXECUTE_WORKFLOW_ID.to_string(),
            current_phase_id: current_phase_id.to_string(),
            artifact_refs: BTreeMap::new(),
            bridge_state: None,
            updated_at: Utc
                .with_ymd_and_hms(2026, 4, 21, 9, 0, 0)
                .single()
                .expect("datetime should be valid"),
        }
    }

    #[test]
    fn planning_workflow_state_skips_missing_plan_artifact() {
        let (_guard, working_dir) = prepare_working_dir();

        let state = bootstrap_plan_workflow_state(
            "session-a",
            &working_dir,
            &astrcode_core::ModeId::plan(),
        )
        .expect("bootstrap should succeed")
        .unwrap_or_else(|| panic!("plan mode should bootstrap planning state"));

        assert!(
            !state.artifact_refs.contains_key("canonical-plan"),
            "missing markdown file should not produce phantom artifact ref"
        );
    }

    #[test]
    fn advance_plan_workflow_to_execution_returns_none_without_plan_state() {
        let (_guard, working_dir) = prepare_working_dir();

        let next = advance_plan_workflow_to_execution("session-a", &working_dir)
            .expect("missing plan state should not fail");

        assert!(next.is_none());
    }

    #[test]
    fn advance_plan_workflow_to_execution_returns_none_when_plan_is_not_reviewable() {
        let (_guard, working_dir) = prepare_working_dir();
        persist_plan_fixture("session-a", &working_dir, SessionPlanStatus::Draft, true);

        let next = advance_plan_workflow_to_execution("session-a", &working_dir)
            .expect("draft plan should not fail");

        assert!(next.is_none());
    }

    #[test]
    fn advance_plan_workflow_to_execution_rejects_missing_approved_plan_artifact() {
        let (_guard, working_dir) = prepare_working_dir();
        persist_plan_fixture(
            "session-a",
            &working_dir,
            SessionPlanStatus::Approved,
            false,
        );

        let error = advance_plan_workflow_to_execution("session-a", &working_dir)
            .expect_err("approved plan without markdown should fail");

        assert!(matches!(error, ApplicationError::Internal(_)));
        assert!(error.to_string().contains("approved plan artifact"));
    }

    #[test]
    fn revert_execution_to_planning_workflow_state_restores_canonical_plan_reference() {
        let (_guard, working_dir) = prepare_working_dir();
        let state =
            persist_plan_fixture("session-a", &working_dir, SessionPlanStatus::Approved, true);

        let planning = revert_execution_to_planning_workflow_state("session-a", &working_dir)
            .expect("reverting workflow state should succeed");

        assert_eq!(planning.workflow_id, PLAN_EXECUTE_WORKFLOW_ID);
        assert_eq!(planning.current_phase_id, PLANNING_PHASE_ID);
        assert!(planning.bridge_state.is_none());
        assert_eq!(
            planning
                .artifact_refs
                .get("canonical-plan")
                .expect("canonical plan should exist")
                .path,
            crate::session_plan::session_plan_markdown_path(
                "session-a",
                &working_dir,
                &state.active_plan_slug
            )
            .expect("plan path should resolve")
            .display()
            .to_string()
        );
    }

    #[test]
    fn extract_implementation_steps_preserves_explicit_numbering() {
        let steps = extract_implementation_steps(
            "# Plan\n\n## 实现步骤\n2. 第二步\n4. 第四步\n- 无序补充\n",
        );

        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].index, 2);
        assert_eq!(steps[0].summary, "第二步");
        assert_eq!(steps[1].index, 4);
        assert_eq!(steps[1].summary, "第四步");
        assert_eq!(steps[2].index, 3);
    }

    #[tokio::test]
    async fn reconcile_workflow_phase_mode_keeps_current_mode_when_phase_already_matches() {
        let (_guard, working_dir) = prepare_working_dir();
        let calls = Arc::new(AtomicUsize::new(0));

        let mode = reconcile_workflow_phase_mode(
            &WorkflowOrchestrator::default(),
            "session-a",
            &working_dir,
            ModeId::plan(),
            &workflow_state(PLANNING_PHASE_ID),
            None,
            |_| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(ApplicationError::Internal(
                        "switch_mode should not be called".to_string(),
                    ))
                }
            },
        )
        .await
        .expect("matching phase mode should succeed");

        assert_eq!(mode, ModeId::plan());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn reconcile_workflow_phase_mode_allows_reviewing_approved_plan_in_code_mode() {
        let (_guard, working_dir) = prepare_working_dir();
        let calls = Arc::new(AtomicUsize::new(0));
        let plan_state = sample_plan_state(SessionPlanStatus::AwaitingApproval);

        let mode = reconcile_workflow_phase_mode(
            &WorkflowOrchestrator::default(),
            "session-a",
            &working_dir,
            ModeId::code(),
            &workflow_state(PLANNING_PHASE_ID),
            Some(&plan_state),
            |_| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(ApplicationError::Internal(
                        "switch_mode should not be called".to_string(),
                    ))
                }
            },
        )
        .await
        .expect("planning review mode should stay in code mode");

        assert_eq!(mode, ModeId::code());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn reconcile_workflow_phase_mode_switches_to_phase_mode_when_needed() {
        let (_guard, working_dir) = prepare_working_dir();
        let requested_modes = Arc::new(Mutex::new(Vec::new()));

        let mode = reconcile_workflow_phase_mode(
            &WorkflowOrchestrator::default(),
            "session-a",
            &working_dir,
            ModeId::plan(),
            &workflow_state(EXECUTING_PHASE_ID),
            None,
            |target_mode| {
                let requested_modes = Arc::clone(&requested_modes);
                async move {
                    requested_modes
                        .lock()
                        .expect("requested mode lock should work")
                        .push(target_mode.clone());
                    Ok(SessionModeSnapshot {
                        current_mode_id: target_mode,
                        last_mode_changed_at: None,
                    })
                }
            },
        )
        .await
        .expect("mode reconcile should switch to executing mode");

        assert_eq!(mode, ModeId::code());
        assert_eq!(
            requested_modes
                .lock()
                .expect("requested mode lock should work")
                .as_slice(),
            &[ModeId::code()]
        );
    }
}
