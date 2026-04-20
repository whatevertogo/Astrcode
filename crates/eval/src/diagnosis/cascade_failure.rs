use serde_json::json;

use super::{FailureInstance, FailurePatternDetector, FailureSeverity};
use crate::trace::{StorageSeqRange, TurnTrace};

#[derive(Default)]
pub struct CascadeFailureDetector;

impl FailurePatternDetector for CascadeFailureDetector {
    fn name(&self) -> &'static str {
        "cascade_failure"
    }

    fn severity(&self) -> FailureSeverity {
        FailureSeverity::High
    }

    fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance> {
        let mut failures = Vec::new();
        let mut index = 0;
        while index < trace.tool_calls.len() {
            if trace.tool_calls[index].success != Some(false) {
                index += 1;
                continue;
            }

            let start = index;
            while index < trace.tool_calls.len() && trace.tool_calls[index].success == Some(false) {
                index += 1;
            }
            let run = &trace.tool_calls[start..index];
            if run.len() < 2 {
                continue;
            }

            let recovered_by_retry = trace.tool_calls.get(index).is_some_and(|next| {
                next.success == Some(true)
                    && run.iter().all(|call| call.tool_name == next.tool_name)
            });
            if recovered_by_retry {
                continue;
            }

            let seqs: Vec<u64> = run
                .iter()
                .flat_map(|call| [call.started_storage_seq, call.finished_storage_seq])
                .flatten()
                .collect();
            let range = seqs
                .first()
                .zip(seqs.last())
                .map(|(first, last)| StorageSeqRange {
                    start: *first,
                    end: *last,
                });

            failures.push(FailureInstance {
                pattern_name: self.name().to_string(),
                severity: self.severity(),
                confidence: 0.85,
                storage_seq_range: range,
                description: format!("检测到连续 {} 次工具失败", run.len()),
                context: Some(json!({
                    "tools": run.iter().map(|call| {
                        json!({
                            "toolName": call.tool_name,
                            "toolCallId": call.tool_call_id,
                            "error": call.error,
                        })
                    }).collect::<Vec<_>>()
                })),
            });
        }
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::CascadeFailureDetector;
    use crate::{
        diagnosis::FailurePatternDetector,
        trace::{ToolCallRecord, TurnTrace},
    };

    fn tool_call(name: &str, success: bool, error: Option<&str>) -> ToolCallRecord {
        ToolCallRecord {
            tool_call_id: format!("{name}-{}", success as u8),
            tool_name: name.to_string(),
            args: serde_json::Value::Null,
            output: None,
            success: Some(success),
            error: error.map(str::to_string),
            metadata: None,
            continuation: None,
            duration_ms: None,
            started_storage_seq: None,
            finished_storage_seq: None,
            stream_deltas: Vec::new(),
            persisted_reference: None,
        }
    }

    fn turn(calls: Vec<ToolCallRecord>) -> TurnTrace {
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

    #[test]
    fn detector_reports_consecutive_failures() {
        let detector = CascadeFailureDetector;
        let failures = detector.detect(&turn(vec![
            tool_call("Read", false, Some("missing")),
            tool_call("Edit", false, Some("locked")),
        ]));
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn detector_ignores_single_failure() {
        let detector = CascadeFailureDetector;
        let failures = detector.detect(&turn(vec![tool_call("Read", false, Some("missing"))]));
        assert!(failures.is_empty());
    }

    #[test]
    fn detector_ignores_failure_then_retry_success() {
        let detector = CascadeFailureDetector;
        let failures = detector.detect(&turn(vec![
            tool_call("Read", false, Some("missing")),
            tool_call("Read", false, Some("missing")),
            tool_call("Read", true, None),
        ]));
        assert!(failures.is_empty());
    }
}
