use std::{collections::BTreeMap, path::Path};

use astrcode_core::{WorkflowDef, WorkflowPhaseDef, WorkflowSignal, WorkflowTransitionDef};

use crate::{
    ApplicationError,
    workflow::{
        bridge::PlanToExecuteBridgeState,
        definition::{
            EXECUTING_PHASE_ID, PLAN_EXECUTE_WORKFLOW_ID, PLANNING_PHASE_ID, builtin_workflows,
        },
        state::{WorkflowInstanceState, WorkflowStateService},
    },
};

/// application 层唯一的 workflow 编排入口。
///
/// Why: 正式 workflow 的 phase 图、恢复与迁移查询不应继续散落在 plan-specific if/else 中。
#[derive(Debug, Clone)]
pub struct WorkflowOrchestrator {
    workflows: BTreeMap<String, WorkflowDef>,
}

impl Default for WorkflowOrchestrator {
    fn default() -> Self {
        Self::new(builtin_workflows())
    }
}

impl WorkflowOrchestrator {
    pub fn new(workflows: Vec<WorkflowDef>) -> Self {
        Self {
            workflows: workflows
                .into_iter()
                .map(|workflow| (workflow.workflow_id.clone(), workflow))
                .collect(),
        }
    }

    pub fn workflow(&self, workflow_id: &str) -> Option<&WorkflowDef> {
        self.workflows.get(workflow_id)
    }

    pub fn phase<'a>(
        &'a self,
        state: &WorkflowInstanceState,
    ) -> Result<&'a WorkflowPhaseDef, ApplicationError> {
        let workflow = self.workflow(&state.workflow_id).ok_or_else(|| {
            ApplicationError::Internal(format!(
                "workflow '{}' is not registered",
                state.workflow_id
            ))
        })?;
        workflow
            .phases
            .iter()
            .find(|phase| phase.phase_id == state.current_phase_id)
            .ok_or_else(|| {
                ApplicationError::Internal(format!(
                    "workflow '{}' does not contain phase '{}'",
                    state.workflow_id, state.current_phase_id
                ))
            })
    }

    pub fn transition_for_signal<'a>(
        &'a self,
        state: &WorkflowInstanceState,
        signal: WorkflowSignal,
    ) -> Result<Option<&'a WorkflowTransitionDef>, ApplicationError> {
        let workflow = self.workflow(&state.workflow_id).ok_or_else(|| {
            ApplicationError::Internal(format!(
                "workflow '{}' is not registered",
                state.workflow_id
            ))
        })?;
        Ok(workflow.transitions.iter().find(|transition| {
            transition.source_phase_id == state.current_phase_id
                && matches!(
                    transition.trigger,
                    astrcode_core::WorkflowTransitionTrigger::Signal {
                        signal: transition_signal,
                    } if transition_signal == signal
                )
        }))
    }

    pub fn load_active_workflow(
        &self,
        session_id: &str,
        working_dir: &Path,
    ) -> Result<Option<WorkflowInstanceState>, ApplicationError> {
        let Some(state) = WorkflowStateService::load_recovering(session_id, working_dir)? else {
            return Ok(None);
        };
        if let Err(error) = self.validate_state(&state) {
            let path = WorkflowStateService::state_path(session_id, working_dir)?;
            log::warn!(
                "workflow state '{}' is invalid and will degrade to mode-only: {}",
                path.display(),
                error
            );
            return Ok(None);
        }
        Ok(Some(state))
    }

    pub fn persist_active_workflow(
        &self,
        session_id: &str,
        working_dir: &Path,
        state: &WorkflowInstanceState,
    ) -> Result<(), ApplicationError> {
        self.validate_state(state)?;
        WorkflowStateService::persist(session_id, working_dir, state)
    }

    pub fn clear_active_workflow(
        &self,
        session_id: &str,
        working_dir: &Path,
    ) -> Result<(), ApplicationError> {
        WorkflowStateService::clear(session_id, working_dir)
    }

    fn validate_state(&self, state: &WorkflowInstanceState) -> Result<(), ApplicationError> {
        let phase = self.phase(state)?;
        match (state.workflow_id.as_str(), phase.phase_id.as_str()) {
            (PLAN_EXECUTE_WORKFLOW_ID, PLANNING_PHASE_ID) => {
                if state.bridge_state.is_some() {
                    return Err(ApplicationError::Internal(
                        "planning workflow state must not carry execute bridge state".to_string(),
                    ));
                }
            },
            (PLAN_EXECUTE_WORKFLOW_ID, EXECUTING_PHASE_ID) => {
                let bridge_state = state.bridge_state.as_ref().ok_or_else(|| {
                    ApplicationError::Internal(
                        "executing workflow state must include plan execute bridge state"
                            .to_string(),
                    )
                })?;
                if bridge_state.source_phase_id != PLANNING_PHASE_ID
                    || bridge_state.target_phase_id != EXECUTING_PHASE_ID
                {
                    return Err(ApplicationError::Internal(format!(
                        "unexpected plan execute bridge transition '{} -> {}'",
                        bridge_state.source_phase_id, bridge_state.target_phase_id
                    )));
                }
                PlanToExecuteBridgeState::from_bridge_state(bridge_state)?;
            },
            _ => {},
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use astrcode_core::WorkflowSignal;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::WorkflowOrchestrator;
    use crate::workflow::{
        bridge::{PlanImplementationStep, PlanToExecuteBridgeState},
        definition::{EXECUTING_PHASE_ID, PLANNING_PHASE_ID},
        state::{WorkflowArtifactRef, WorkflowInstanceState, WorkflowStateService},
    };

    fn workflow_state() -> WorkflowInstanceState {
        let plan_artifact = WorkflowArtifactRef {
            artifact_kind: "canonical-plan".to_string(),
            path: "/tmp/plan.md".to_string(),
            content_digest: Some("abc".to_string()),
        };
        let bridge = PlanToExecuteBridgeState {
            plan_artifact: plan_artifact.clone(),
            plan_title: "Cleanup runtime".to_string(),
            implementation_steps: vec![PlanImplementationStep {
                index: 1,
                title: "Refactor".to_string(),
                summary: "收拢 workflow state".to_string(),
            }],
            approved_at: Some(
                Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0)
                    .single()
                    .expect("datetime should be valid"),
            ),
        };
        WorkflowInstanceState {
            workflow_id: "plan_execute".to_string(),
            current_phase_id: EXECUTING_PHASE_ID.to_string(),
            artifact_refs: BTreeMap::from([("canonical-plan".to_string(), plan_artifact)]),
            bridge_state: Some(
                bridge
                    .into_bridge_state(PLANNING_PHASE_ID, EXECUTING_PHASE_ID)
                    .expect("bridge should encode"),
            ),
            updated_at: Utc
                .with_ymd_and_hms(2026, 4, 21, 12, 1, 0)
                .single()
                .expect("datetime should be valid"),
        }
    }

    #[test]
    fn load_active_workflow_returns_registered_state() {
        let guard = astrcode_core::test_support::TestEnvGuard::new();
        let working_dir = guard.home_dir().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should exist");
        let orchestrator = WorkflowOrchestrator::default();
        let state = workflow_state();

        orchestrator
            .persist_active_workflow("session-a", &working_dir, &state)
            .expect("state should persist");

        let loaded = orchestrator
            .load_active_workflow("session-a", &working_dir)
            .expect("state should load")
            .expect("workflow should exist");

        assert_eq!(loaded, state);
        let transition = orchestrator
            .transition_for_signal(&loaded, WorkflowSignal::Replan)
            .expect("transition lookup should succeed")
            .expect("replan transition should exist");
        assert_eq!(transition.target_phase_id, PLANNING_PHASE_ID);
    }

    #[test]
    fn load_active_workflow_downgrades_unknown_phase() {
        let guard = astrcode_core::test_support::TestEnvGuard::new();
        let working_dir = guard.home_dir().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should exist");
        let state = WorkflowInstanceState {
            current_phase_id: "unknown".to_string(),
            ..workflow_state()
        };
        WorkflowStateService::persist("session-a", &working_dir, &state)
            .expect("state should persist");

        let loaded = WorkflowOrchestrator::default()
            .load_active_workflow("session-a", &working_dir)
            .expect("recovery should not fail");
        assert!(
            loaded.is_none(),
            "unknown phase should downgrade to mode-only"
        );
    }

    #[test]
    fn load_active_workflow_downgrades_invalid_execute_bridge() {
        let guard = astrcode_core::test_support::TestEnvGuard::new();
        let working_dir = guard.home_dir().join("workspace");
        fs::create_dir_all(&working_dir).expect("workspace should exist");
        let state = WorkflowInstanceState {
            bridge_state: Some(astrcode_core::WorkflowBridgeState {
                bridge_kind: "noop".to_string(),
                source_phase_id: PLANNING_PHASE_ID.to_string(),
                target_phase_id: EXECUTING_PHASE_ID.to_string(),
                schema_version: 1,
                payload: json!({}),
            }),
            ..workflow_state()
        };
        WorkflowStateService::persist("session-a", &working_dir, &state)
            .expect("state should persist");

        let loaded = WorkflowOrchestrator::default()
            .load_active_workflow("session-a", &working_dir)
            .expect("recovery should not fail");
        assert!(
            loaded.is_none(),
            "invalid execute bridge should downgrade to mode-only"
        );
    }

    #[test]
    fn transition_lookup_returns_none_when_signal_is_not_declared() {
        let orchestrator = WorkflowOrchestrator::default();
        let state = WorkflowInstanceState {
            current_phase_id: PLANNING_PHASE_ID.to_string(),
            bridge_state: Some(astrcode_core::WorkflowBridgeState {
                bridge_kind: "noop".to_string(),
                source_phase_id: PLANNING_PHASE_ID.to_string(),
                target_phase_id: EXECUTING_PHASE_ID.to_string(),
                schema_version: 1,
                payload: json!({}),
            }),
            ..workflow_state()
        };

        let transition = orchestrator
            .transition_for_signal(&state, WorkflowSignal::Replan)
            .expect("transition lookup should succeed");
        assert!(transition.is_none());
    }
}
