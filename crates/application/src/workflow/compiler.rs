use std::collections::BTreeMap;

use astrcode_core::{
    WorkflowDef, WorkflowPhaseDef, WorkflowSignal, WorkflowTransitionDef, WorkflowTransitionTrigger,
};

use crate::ApplicationError;

/// 经过显式校验的 workflow 定义。
///
/// Why: orchestrator 不应再直接消费“未经校验的 DTO”，
/// 否则 phase 图、signal 契约和 phase -> mode 绑定仍会在运行时分散失败。
/// 当前 phase / transition 数量很小，compile 之后继续保留顺序容器即可；
/// 这里刻意不引入额外索引结构，避免为了理论规模过度设计。
#[derive(Debug, Clone)]
pub(crate) struct CompiledWorkflowDef {
    definition: WorkflowDef,
}

impl CompiledWorkflowDef {
    pub(crate) fn compile(definition: WorkflowDef) -> Result<Self, ApplicationError> {
        validate_workflow_definition(&definition)?;
        Ok(Self { definition })
    }

    pub(crate) fn definition(&self) -> &WorkflowDef {
        &self.definition
    }

    pub(crate) fn phase(&self, phase_id: &str) -> Option<&WorkflowPhaseDef> {
        self.definition
            .phases
            .iter()
            .find(|phase| phase.phase_id == phase_id)
    }

    pub(crate) fn transition_for_signal(
        &self,
        source_phase_id: &str,
        signal: WorkflowSignal,
    ) -> Option<&WorkflowTransitionDef> {
        self.definition.transitions.iter().find(|transition| {
            transition.source_phase_id == source_phase_id
                && matches!(
                    transition.trigger,
                    WorkflowTransitionTrigger::Signal {
                        signal: transition_signal,
                    } if transition_signal == signal
                )
        })
    }
}

pub(crate) fn compile_workflows(
    workflows: Vec<WorkflowDef>,
) -> Result<BTreeMap<String, CompiledWorkflowDef>, ApplicationError> {
    let mut compiled = BTreeMap::new();
    for workflow in workflows {
        let compiled_workflow = CompiledWorkflowDef::compile(workflow)?;
        let workflow_id = compiled_workflow.definition().workflow_id.clone();
        if compiled.contains_key(&workflow_id) {
            return Err(ApplicationError::Internal(format!(
                "duplicate workflow id '{}'",
                workflow_id
            )));
        }
        compiled.insert(workflow_id, compiled_workflow);
    }
    Ok(compiled)
}

fn validate_workflow_definition(workflow: &WorkflowDef) -> Result<(), ApplicationError> {
    if workflow.workflow_id.trim().is_empty() {
        return Err(ApplicationError::Internal(
            "workflow id must not be empty".to_string(),
        ));
    }
    if workflow.initial_phase_id.trim().is_empty() {
        return Err(ApplicationError::Internal(format!(
            "workflow '{}' must declare initial phase id",
            workflow.workflow_id
        )));
    }
    if workflow.phases.is_empty() {
        return Err(ApplicationError::Internal(format!(
            "workflow '{}' must declare at least one phase",
            workflow.workflow_id
        )));
    }

    let mut phases = BTreeMap::<&str, &WorkflowPhaseDef>::new();
    for phase in &workflow.phases {
        if phase.phase_id.trim().is_empty() {
            return Err(ApplicationError::Internal(format!(
                "workflow '{}' contains phase with empty id",
                workflow.workflow_id
            )));
        }
        if phase.mode_id.as_str().trim().is_empty() {
            return Err(ApplicationError::Internal(format!(
                "workflow '{}' phase '{}' must declare mode_id",
                workflow.workflow_id, phase.phase_id
            )));
        }
        if phases.insert(phase.phase_id.as_str(), phase).is_some() {
            return Err(ApplicationError::Internal(format!(
                "workflow '{}' contains duplicate phase '{}'",
                workflow.workflow_id, phase.phase_id
            )));
        }
    }

    if !phases.contains_key(workflow.initial_phase_id.as_str()) {
        return Err(ApplicationError::Internal(format!(
            "workflow '{}' initial phase '{}' is not declared",
            workflow.workflow_id, workflow.initial_phase_id
        )));
    }

    let mut transitions = BTreeMap::<&str, &WorkflowTransitionDef>::new();
    for transition in &workflow.transitions {
        if transition.transition_id.trim().is_empty() {
            return Err(ApplicationError::Internal(format!(
                "workflow '{}' contains transition with empty id",
                workflow.workflow_id
            )));
        }
        if transitions
            .insert(transition.transition_id.as_str(), transition)
            .is_some()
        {
            return Err(ApplicationError::Internal(format!(
                "workflow '{}' contains duplicate transition '{}'",
                workflow.workflow_id, transition.transition_id
            )));
        }
        let Some(source_phase) = phases.get(transition.source_phase_id.as_str()) else {
            return Err(ApplicationError::Internal(format!(
                "workflow '{}' transition '{}' references unknown source phase '{}'",
                workflow.workflow_id, transition.transition_id, transition.source_phase_id
            )));
        };
        if !phases.contains_key(transition.target_phase_id.as_str()) {
            return Err(ApplicationError::Internal(format!(
                "workflow '{}' transition '{}' references unknown target phase '{}'",
                workflow.workflow_id, transition.transition_id, transition.target_phase_id
            )));
        }
        if let WorkflowTransitionTrigger::Signal { signal } = transition.trigger {
            if !source_phase.accepted_signals.contains(&signal) {
                return Err(ApplicationError::Internal(format!(
                    "workflow '{}' transition '{}' uses signal '{signal:?}' not accepted by phase \
                     '{}'",
                    workflow.workflow_id, transition.transition_id, transition.source_phase_id
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use astrcode_core::{ModeId, WorkflowSignal, WorkflowTransitionTrigger};

    use super::{CompiledWorkflowDef, compile_workflows};

    fn valid_workflow() -> astrcode_core::WorkflowDef {
        astrcode_core::WorkflowDef {
            workflow_id: "plan_execute".to_string(),
            initial_phase_id: "planning".to_string(),
            phases: vec![
                astrcode_core::WorkflowPhaseDef {
                    phase_id: "planning".to_string(),
                    mode_id: ModeId::plan(),
                    role: "planning".to_string(),
                    artifact_kind: Some("canonical-plan".to_string()),
                    accepted_signals: vec![WorkflowSignal::Approve],
                },
                astrcode_core::WorkflowPhaseDef {
                    phase_id: "executing".to_string(),
                    mode_id: ModeId::code(),
                    role: "executing".to_string(),
                    artifact_kind: Some("execution-bridge".to_string()),
                    accepted_signals: vec![WorkflowSignal::Replan],
                },
            ],
            transitions: vec![astrcode_core::WorkflowTransitionDef {
                transition_id: "plan-approved".to_string(),
                source_phase_id: "planning".to_string(),
                target_phase_id: "executing".to_string(),
                trigger: WorkflowTransitionTrigger::Signal {
                    signal: WorkflowSignal::Approve,
                },
            }],
        }
    }

    #[test]
    fn compile_workflow_accepts_valid_phase_graph() {
        let compiled = CompiledWorkflowDef::compile(valid_workflow()).expect("workflow compiles");

        assert_eq!(compiled.definition().workflow_id, "plan_execute");
        assert_eq!(
            compiled
                .phase("planning")
                .expect("planning phase should exist")
                .mode_id,
            ModeId::plan()
        );
    }

    #[test]
    fn compile_workflow_rejects_unknown_initial_phase() {
        let mut workflow = valid_workflow();
        workflow.initial_phase_id = "missing".to_string();

        let error =
            CompiledWorkflowDef::compile(workflow).expect_err("missing initial phase must fail");

        assert!(
            error
                .to_string()
                .contains("initial phase 'missing' is not declared")
        );
    }

    #[test]
    fn compile_workflow_rejects_signal_transition_not_accepted_by_phase() {
        let mut workflow = valid_workflow();
        workflow.phases[0].accepted_signals.clear();

        let error =
            CompiledWorkflowDef::compile(workflow).expect_err("undeclared phase signal must fail");

        assert!(
            error
                .to_string()
                .contains("uses signal 'Approve' not accepted by phase 'planning'")
        );
    }

    #[test]
    fn compile_workflows_rejects_duplicate_workflow_ids() {
        let error = compile_workflows(vec![valid_workflow(), valid_workflow()])
            .expect_err("duplicate workflow ids must fail");

        assert!(
            error
                .to_string()
                .contains("duplicate workflow id 'plan_execute'")
        );
    }
}
