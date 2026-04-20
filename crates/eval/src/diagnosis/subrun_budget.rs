use serde_json::json;

use super::{FailureInstance, FailurePatternDetector, FailureSeverity};
use crate::trace::TurnTrace;

#[derive(Default)]
pub struct SubRunBudgetDetector;

impl FailurePatternDetector for SubRunBudgetDetector {
    fn name(&self) -> &'static str {
        "subrun_budget"
    }

    fn severity(&self) -> FailureSeverity {
        FailureSeverity::Medium
    }

    fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance> {
        trace
            .sub_runs
            .iter()
            .filter_map(|sub_run| {
                let max_steps = sub_run.resolved_limits.max_steps?;
                let actual = sub_run.step_count?;
                if actual <= max_steps {
                    return None;
                }

                Some(FailureInstance {
                    pattern_name: self.name().to_string(),
                    severity: self.severity(),
                    confidence: 0.9,
                    storage_seq_range: sub_run.storage_seq_range.clone(),
                    description: format!(
                        "子 Agent {} 超出步数限制：{} > {}",
                        sub_run.sub_run_id, actual, max_steps
                    ),
                    context: Some(json!({
                        "subRunId": sub_run.sub_run_id,
                        "actualSteps": actual,
                        "maxSteps": max_steps,
                    })),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::SubRunBudgetDetector;
    use crate::{
        diagnosis::FailurePatternDetector,
        trace::{SubRunTrace, TurnTrace},
    };

    fn turn(sub_run: SubRunTrace) -> TurnTrace {
        TurnTrace {
            turn_id: "turn-1".to_string(),
            user_input: None,
            assistant_output: None,
            assistant_reasoning: None,
            thinking_deltas: Vec::new(),
            tool_calls: Vec::new(),
            prompt_metrics: Vec::new(),
            compactions: Vec::new(),
            sub_runs: vec![sub_run],
            collaboration_facts: Vec::new(),
            errors: Vec::new(),
            timeline: Vec::new(),
            agent_lineage: Vec::new(),
            storage_seq_range: None,
            completed_at: None,
            completion_reason: None,
            incomplete: false,
        }
    }

    #[test]
    fn detector_reports_subrun_budget_overflow() {
        let detector = SubRunBudgetDetector;
        let failures = detector.detect(&turn(SubRunTrace {
            sub_run_id: "sub-1".to_string(),
            tool_call_id: None,
            agent_id: None,
            agent_profile: None,
            parent_turn_id: None,
            parent_sub_run_id: None,
            child_session_id: None,
            storage_mode: None,
            resolved_overrides: None,
            resolved_limits: astrcode_core::ResolvedExecutionLimitsSnapshot {
                allowed_tools: Vec::new(),
                max_steps: Some(2),
            },
            started_at: None,
            finished_at: None,
            duration_ms: None,
            step_count: Some(4),
            estimated_tokens: None,
            result: None,
            collaboration_facts: Vec::new(),
            storage_seq_range: None,
        }));
        assert_eq!(failures.len(), 1);
    }
}
