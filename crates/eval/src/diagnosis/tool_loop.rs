use serde_json::json;

use super::{FailureInstance, FailurePatternDetector, FailureSeverity};
use crate::trace::{StorageSeqRange, TurnTrace};

pub struct ToolLoopDetector {
    pub min_repetitions: usize,
    pub similarity_threshold: f64,
}

impl Default for ToolLoopDetector {
    fn default() -> Self {
        Self {
            min_repetitions: 3,
            similarity_threshold: 0.7,
        }
    }
}

impl FailurePatternDetector for ToolLoopDetector {
    fn name(&self) -> &'static str {
        "tool_loop"
    }

    fn severity(&self) -> FailureSeverity {
        FailureSeverity::High
    }

    fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance> {
        let mut current_run = Vec::new();
        let mut failures = Vec::new();

        for call in &trace.tool_calls {
            if current_run.is_empty()
                || current_run
                    .last()
                    .is_some_and(|prev: &&crate::trace::ToolCallRecord| {
                        prev.tool_name == call.tool_name
                    })
            {
                current_run.push(call);
            } else {
                if let Some(instance) = self.evaluate_run(&current_run) {
                    failures.push(instance);
                }
                current_run = vec![call];
            }
        }

        if let Some(instance) = self.evaluate_run(&current_run) {
            failures.push(instance);
        }

        failures
    }
}

impl ToolLoopDetector {
    fn evaluate_run(&self, calls: &[&crate::trace::ToolCallRecord]) -> Option<FailureInstance> {
        if calls.len() < self.min_repetitions {
            return None;
        }

        let args: Vec<String> = calls
            .iter()
            .map(|call| serde_json::to_string(&call.args).unwrap_or_default())
            .collect();
        let similarities: Vec<f64> = args
            .windows(2)
            .map(|pair| jaccard_similarity(&pair[0], &pair[1]))
            .collect();
        if similarities
            .iter()
            .any(|score| *score < self.similarity_threshold)
        {
            return None;
        }

        let seqs: Vec<u64> = calls
            .iter()
            .flat_map(|call| [call.started_storage_seq, call.finished_storage_seq])
            .flatten()
            .collect();
        let range = seqs
            .first()
            .zip(seqs.last())
            .map(|(start, end)| StorageSeqRange {
                start: *start,
                end: *end,
            });

        Some(FailureInstance {
            pattern_name: self.name().to_string(),
            severity: self.severity(),
            confidence: 0.9,
            storage_seq_range: range,
            description: format!(
                "检测到 {} 连续重复调用 {} 次",
                calls[0].tool_name,
                calls.len()
            ),
            context: Some(json!({
                "toolCallIds": calls.iter().map(|call| call.tool_call_id.clone()).collect::<Vec<_>>(),
                "similarities": similarities,
            })),
        })
    }
}

fn jaccard_similarity(left: &str, right: &str) -> f64 {
    let left_tokens = tokenize(left);
    let right_tokens = tokenize(right);
    if left_tokens.is_empty() && right_tokens.is_empty() {
        return 1.0;
    }
    let intersection = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn tokenize(value: &str) -> std::collections::BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ToolLoopDetector;
    use crate::{
        diagnosis::FailurePatternDetector,
        trace::{ToolCallRecord, TurnTrace},
    };

    fn turn_with_calls(calls: Vec<ToolCallRecord>) -> TurnTrace {
        TurnTrace {
            turn_id: "turn-1".to_string(),
            user_input: None,
            assistant_output: None,
            assistant_reasoning: None,
            thinking_deltas: Vec::new(),
            tool_calls: calls,
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
        }
    }

    fn tool_call(id: usize, args: serde_json::Value) -> ToolCallRecord {
        ToolCallRecord {
            tool_call_id: format!("call-{id}"),
            tool_name: "Read".to_string(),
            args,
            output: None,
            success: Some(true),
            error: None,
            metadata: None,
            continuation: None,
            duration_ms: None,
            started_storage_seq: Some(id as u64),
            finished_storage_seq: Some(id as u64 + 10),
            stream_deltas: Vec::new(),
            persisted_reference: None,
        }
    }

    #[test]
    fn detector_reports_similar_repeated_tool_calls() {
        let trace = turn_with_calls(vec![
            tool_call(1, json!({"path":"a.txt"})),
            tool_call(2, json!({"path":"a.txt"})),
            tool_call(3, json!({"path":"a.txt"})),
        ]);
        let detector = ToolLoopDetector::default();
        assert_eq!(detector.detect(&trace).len(), 1);
    }

    #[test]
    fn detector_ignores_same_tool_with_different_args() {
        let trace = turn_with_calls(vec![
            tool_call(1, json!({"path":"a.txt"})),
            tool_call(2, json!({"path":"b.txt"})),
            tool_call(3, json!({"path":"c.txt"})),
        ]);
        let detector = ToolLoopDetector::default();
        assert!(detector.detect(&trace).is_empty());
    }
}
