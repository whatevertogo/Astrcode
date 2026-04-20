use serde_json::json;

use super::{FailureInstance, FailurePatternDetector, FailureSeverity};
use crate::trace::TurnTrace;

pub struct EmptyTurnDetector {
    pub min_output_len: usize,
}

impl Default for EmptyTurnDetector {
    fn default() -> Self {
        Self { min_output_len: 8 }
    }
}

impl FailurePatternDetector for EmptyTurnDetector {
    fn name(&self) -> &'static str {
        "empty_turn"
    }

    fn severity(&self) -> FailureSeverity {
        FailureSeverity::Medium
    }

    fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance> {
        if trace.incomplete {
            return Vec::new();
        }
        let output_len = trace.assistant_output.as_deref().unwrap_or("").trim().len();
        if trace.tool_calls.is_empty() && output_len < self.min_output_len {
            return vec![FailureInstance {
                pattern_name: self.name().to_string(),
                severity: self.severity(),
                confidence: 0.9,
                storage_seq_range: trace.storage_seq_range.clone(),
                description: "turn 完成但没有工具调用且输出为空".to_string(),
                context: Some(json!({
                    "outputLength": output_len,
                    "threshold": self.min_output_len,
                })),
            }];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::EmptyTurnDetector;
    use crate::{diagnosis::FailurePatternDetector, trace::TurnTrace};

    #[test]
    fn detector_reports_empty_turn() {
        let detector = EmptyTurnDetector::default();
        let failures = detector.detect(&TurnTrace {
            turn_id: "turn-1".to_string(),
            user_input: None,
            assistant_output: Some(String::new()),
            assistant_reasoning: None,
            thinking_deltas: Vec::new(),
            tool_calls: Vec::new(),
            prompt_metrics: Vec::new(),
            compactions: Vec::new(),
            sub_runs: Vec::new(),
            collaboration_facts: Vec::new(),
            errors: Vec::new(),
            timeline: Vec::new(),
            agent_lineage: Vec::new(),
            storage_seq_range: None,
            completed_at: None,
            completion_reason: None,
            incomplete: false,
        });
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn detector_ignores_text_only_turn_with_meaningful_output() {
        let detector = EmptyTurnDetector::default();
        let failures = detector.detect(&TurnTrace {
            turn_id: "turn-1".to_string(),
            user_input: None,
            assistant_output: Some("这里是有效回复".to_string()),
            assistant_reasoning: None,
            thinking_deltas: Vec::new(),
            tool_calls: Vec::new(),
            prompt_metrics: Vec::new(),
            compactions: Vec::new(),
            sub_runs: Vec::new(),
            collaboration_facts: Vec::new(),
            errors: Vec::new(),
            timeline: Vec::new(),
            agent_lineage: Vec::new(),
            storage_seq_range: None,
            completed_at: None,
            completion_reason: None,
            incomplete: false,
        });
        assert!(failures.is_empty());
    }
}
