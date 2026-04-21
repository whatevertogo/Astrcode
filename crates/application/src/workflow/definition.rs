use astrcode_core::{
    ModeId, WorkflowDef, WorkflowPhaseDef, WorkflowSignal, WorkflowTransitionDef,
    WorkflowTransitionTrigger,
};

pub const PLAN_EXECUTE_WORKFLOW_ID: &str = "plan_execute";
pub const PLANNING_PHASE_ID: &str = "planning";
pub const EXECUTING_PHASE_ID: &str = "executing";

pub(crate) fn builtin_workflows() -> Vec<WorkflowDef> {
    vec![plan_execute_workflow()]
}

pub fn plan_execute_workflow() -> WorkflowDef {
    WorkflowDef {
        workflow_id: PLAN_EXECUTE_WORKFLOW_ID.to_string(),
        initial_phase_id: PLANNING_PHASE_ID.to_string(),
        phases: vec![
            WorkflowPhaseDef {
                phase_id: PLANNING_PHASE_ID.to_string(),
                mode_id: ModeId::plan(),
                role: "planning".to_string(),
                artifact_kind: Some("canonical-plan".to_string()),
                accepted_signals: vec![
                    WorkflowSignal::Approve,
                    WorkflowSignal::RequestChanges,
                    WorkflowSignal::Cancel,
                ],
            },
            WorkflowPhaseDef {
                phase_id: EXECUTING_PHASE_ID.to_string(),
                mode_id: ModeId::code(),
                role: "executing".to_string(),
                artifact_kind: Some("execution-bridge".to_string()),
                accepted_signals: vec![WorkflowSignal::Replan, WorkflowSignal::Cancel],
            },
        ],
        transitions: vec![
            WorkflowTransitionDef {
                transition_id: "plan-approved".to_string(),
                source_phase_id: PLANNING_PHASE_ID.to_string(),
                target_phase_id: EXECUTING_PHASE_ID.to_string(),
                trigger: WorkflowTransitionTrigger::Signal {
                    signal: WorkflowSignal::Approve,
                },
            },
            WorkflowTransitionDef {
                transition_id: "execution-replan".to_string(),
                source_phase_id: EXECUTING_PHASE_ID.to_string(),
                target_phase_id: PLANNING_PHASE_ID.to_string(),
                trigger: WorkflowTransitionTrigger::Signal {
                    signal: WorkflowSignal::Replan,
                },
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{ModeId, WorkflowSignal, WorkflowTransitionTrigger};

    use super::{
        EXECUTING_PHASE_ID, PLAN_EXECUTE_WORKFLOW_ID, PLANNING_PHASE_ID, plan_execute_workflow,
    };

    #[test]
    fn builtin_plan_execute_workflow_declares_expected_phase_graph() {
        let workflow = plan_execute_workflow();

        assert_eq!(workflow.workflow_id, PLAN_EXECUTE_WORKFLOW_ID);
        assert_eq!(workflow.initial_phase_id, PLANNING_PHASE_ID);
        assert_eq!(workflow.phases.len(), 2);
        assert_eq!(workflow.transitions.len(), 2);
        assert!(workflow.phases.iter().any(|phase| {
            phase.phase_id == PLANNING_PHASE_ID
                && phase.mode_id == ModeId::plan()
                && phase.accepted_signals.contains(&WorkflowSignal::Approve)
        }));
        assert!(workflow.phases.iter().any(|phase| {
            phase.phase_id == EXECUTING_PHASE_ID
                && phase.mode_id == ModeId::code()
                && phase.accepted_signals.contains(&WorkflowSignal::Replan)
        }));
        assert!(workflow.transitions.iter().any(|transition| {
            transition.source_phase_id == PLANNING_PHASE_ID
                && transition.target_phase_id == EXECUTING_PHASE_ID
                && transition.trigger
                    == WorkflowTransitionTrigger::Signal {
                        signal: WorkflowSignal::Approve,
                    }
        }));
    }
}
