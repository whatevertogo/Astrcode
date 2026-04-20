use serde_json::json;

use super::{FailureInstance, FailurePatternDetector, FailureSeverity};
use crate::trace::{StorageSeqRange, TurnTrace};

pub struct CompactInfoLossDetector {
    keywords: Vec<&'static str>,
}

impl Default for CompactInfoLossDetector {
    fn default() -> Self {
        Self {
            keywords: vec![
                "not found",
                "missing",
                "unknown",
                "cannot locate",
                "不存在",
                "找不到",
            ],
        }
    }
}

impl FailurePatternDetector for CompactInfoLossDetector {
    fn name(&self) -> &'static str {
        "compact_info_loss"
    }

    fn severity(&self) -> FailureSeverity {
        FailureSeverity::Medium
    }

    fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance> {
        let mut failures = Vec::new();
        for compact in &trace.compactions {
            let Some(compact_seq) = compact.storage_seq else {
                continue;
            };
            let next_completed_tool = trace
                .tool_calls
                .iter()
                .filter_map(|call| call.finished_storage_seq.map(|seq| (seq, call)))
                .filter(|(seq, _)| *seq > compact_seq)
                .min_by_key(|(seq, _)| *seq)
                .map(|(_, call)| call);
            let Some(tool_call) = next_completed_tool else {
                continue;
            };
            if tool_call.success == Some(true) {
                continue;
            }
            let diagnostic_text = format!(
                "{} {}",
                tool_call.error.clone().unwrap_or_default(),
                tool_call.output.clone().unwrap_or_default()
            )
            .to_ascii_lowercase();
            if !self
                .keywords
                .iter()
                .any(|keyword| diagnostic_text.contains(keyword))
            {
                continue;
            }

            failures.push(FailureInstance {
                pattern_name: self.name().to_string(),
                severity: self.severity(),
                confidence: 0.75,
                storage_seq_range: tool_call.finished_storage_seq.map(|end| StorageSeqRange {
                    start: compact_seq,
                    end,
                }),
                description: format!(
                    "compact 后紧接着出现疑似信息丢失失败：{}",
                    tool_call.tool_name
                ),
                context: Some(json!({
                    "toolCallId": tool_call.tool_call_id,
                    "preTokens": compact.pre_tokens,
                    "postTokensEstimate": compact.post_tokens_estimate,
                    "tokensFreed": compact.tokens_freed,
                    "error": tool_call.error,
                })),
            });
        }
        failures
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::CompactInfoLossDetector;
    use crate::{
        diagnosis::FailurePatternDetector,
        trace::{CompactTrace, ToolCallRecord, TurnTrace},
    };

    fn turn(tool_call: ToolCallRecord) -> TurnTrace {
        TurnTrace {
            turn_id: "turn-1".to_string(),
            user_input: None,
            assistant_output: None,
            assistant_reasoning: None,
            thinking_deltas: Vec::new(),
            tool_calls: vec![tool_call],
            prompt_metrics: Vec::new(),
            compactions: vec![CompactTrace {
                storage_seq: Some(5),
                agent: Default::default(),
                trigger: astrcode_core::CompactTrigger::Auto,
                summary: "summary".to_string(),
                meta: astrcode_core::CompactAppliedMeta {
                    mode: astrcode_core::CompactMode::Full,
                    instructions_present: false,
                    fallback_used: false,
                    retry_count: 0,
                    input_units: 0,
                    output_summary_chars: 0,
                },
                preserved_recent_turns: 1,
                pre_tokens: 1000,
                post_tokens_estimate: 300,
                messages_removed: 10,
                tokens_freed: 700,
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
            }],
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
    fn detector_reports_compact_followed_by_missing_context_failure() {
        let detector = CompactInfoLossDetector::default();
        let failures = detector.detect(&turn(ToolCallRecord {
            tool_call_id: "call-1".to_string(),
            tool_name: "Read".to_string(),
            args: serde_json::Value::Null,
            output: None,
            success: Some(false),
            error: Some("file not found".to_string()),
            metadata: None,
            continuation: None,
            duration_ms: None,
            started_storage_seq: Some(6),
            finished_storage_seq: Some(7),
            stream_deltas: Vec::new(),
            persisted_reference: None,
        }));
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn detector_ignores_compact_followed_by_success() {
        let detector = CompactInfoLossDetector::default();
        let failures = detector.detect(&turn(ToolCallRecord {
            tool_call_id: "call-1".to_string(),
            tool_name: "Read".to_string(),
            args: serde_json::Value::Null,
            output: Some("ok".to_string()),
            success: Some(true),
            error: None,
            metadata: None,
            continuation: None,
            duration_ms: None,
            started_storage_seq: Some(6),
            finished_storage_seq: Some(7),
            stream_deltas: Vec::new(),
            persisted_reference: None,
        }));
        assert!(failures.is_empty());
    }
}
