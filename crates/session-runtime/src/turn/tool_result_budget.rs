//! request 组装前的 aggregate tool-result budget。
//!
//! Why: 单个工具自己的 inline limit 只能处理“一个工具太大”的情况，
//! 这里负责把同一批 trailing tool results 当作整体治理，并把 replacement
//! 决策收敛到稳定状态里。

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use astrcode_core::{
    LlmMessage, Result, StorageEventPayload, is_persisted_output, persist_tool_result,
};

use crate::{SessionState, turn::events::tool_result_reference_applied_event};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultReplacementRecord {
    pub persisted_relative_path: String,
    pub replacement: String,
    pub original_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ToolResultReplacementState {
    replacements: HashMap<String, ToolResultReplacementRecord>,
    frozen: HashSet<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolResultBudgetStats {
    pub replacement_count: usize,
    pub reapply_count: usize,
    pub bytes_saved: usize,
    pub over_budget_message_count: usize,
}

#[derive(Debug, Clone)]
pub struct ToolResultBudgetOutcome {
    pub messages: Vec<LlmMessage>,
    pub events: Vec<astrcode_core::StorageEvent>,
    pub stats: ToolResultBudgetStats,
}

pub struct ApplyToolResultBudgetRequest<'a> {
    pub messages: &'a [LlmMessage],
    pub session_id: &'a str,
    pub working_dir: &'a Path,
    pub session_state: &'a SessionState,
    pub replacement_state: &'a mut ToolResultReplacementState,
    pub aggregate_budget_bytes: usize,
    pub turn_id: &'a str,
    pub agent: &'a astrcode_core::AgentEventContext,
}

impl ToolResultReplacementState {
    pub fn seed(session_state: &SessionState) -> Result<Self> {
        let mut state = Self::default();
        for stored in session_state.snapshot_recent_stored_events()? {
            if let StorageEventPayload::ToolResultReferenceApplied {
                tool_call_id,
                persisted_relative_path,
                replacement,
                original_bytes,
            } = stored.event.payload
            {
                state.replacements.insert(
                    tool_call_id.clone(),
                    ToolResultReplacementRecord {
                        persisted_relative_path,
                        replacement,
                        original_bytes,
                    },
                );
            }
        }
        Ok(state)
    }

    fn replacement_for(&self, tool_call_id: &str) -> Option<&ToolResultReplacementRecord> {
        self.replacements.get(tool_call_id)
    }

    fn is_frozen(&self, tool_call_id: &str) -> bool {
        self.frozen.contains(tool_call_id)
    }

    fn freeze(&mut self, tool_call_id: String) {
        self.frozen.insert(tool_call_id);
    }

    fn record_replacement(&mut self, tool_call_id: String, record: ToolResultReplacementRecord) {
        self.replacements.insert(tool_call_id.clone(), record);
        self.frozen.remove(&tool_call_id);
    }
}

pub fn apply_tool_result_budget(
    request: ApplyToolResultBudgetRequest<'_>,
) -> Result<ToolResultBudgetOutcome> {
    let mut messages = request.messages.to_vec();
    let mut events = Vec::new();
    let mut stats = ToolResultBudgetStats::default();
    let Some(batch_start) = trailing_tool_batch_start(&messages) else {
        return Ok(ToolResultBudgetOutcome {
            messages,
            events,
            stats,
        });
    };

    let mut total_bytes = 0usize;
    for message in &messages[batch_start..] {
        if let LlmMessage::Tool { content, .. } = message {
            total_bytes = total_bytes.saturating_add(content.len());
        }
    }

    for message in &mut messages[batch_start..] {
        let LlmMessage::Tool {
            tool_call_id,
            content,
        } = message
        else {
            continue;
        };
        if let Some(record) = request.replacement_state.replacement_for(tool_call_id) {
            if content != &record.replacement {
                total_bytes = total_bytes
                    .saturating_sub(content.len())
                    .saturating_add(record.replacement.len());
                *content = record.replacement.clone();
                stats.reapply_count = stats.reapply_count.saturating_add(1);
            }
        }
    }

    if total_bytes <= request.aggregate_budget_bytes {
        return Ok(ToolResultBudgetOutcome {
            messages,
            events,
            stats,
        });
    }
    stats.over_budget_message_count = 1;

    let session_dir = resolve_session_dir(request.working_dir, request.session_id)?;
    let mut fresh_candidates = messages[batch_start..]
        .iter()
        .enumerate()
        .filter_map(|(offset, message)| match message {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } if request
                .replacement_state
                .replacement_for(tool_call_id)
                .is_none()
                && !request.replacement_state.is_frozen(tool_call_id)
                && !is_persisted_output(content) =>
            {
                Some((batch_start + offset, tool_call_id.clone(), content.len()))
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    fresh_candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.2));

    let mut replaced = HashSet::new();
    for (index, tool_call_id, original_len) in fresh_candidates {
        if total_bytes <= request.aggregate_budget_bytes {
            break;
        }
        let LlmMessage::Tool { content, .. } = &messages[index] else {
            continue;
        };
        let replacement = persist_tool_result(&session_dir, &tool_call_id, content);
        let Some(persisted_relative_path) = extract_persisted_relative_path(&replacement) else {
            continue;
        };
        let saved_bytes = original_len.saturating_sub(replacement.len());
        let record = ToolResultReplacementRecord {
            persisted_relative_path: persisted_relative_path.clone(),
            replacement: replacement.clone(),
            original_bytes: original_len as u64,
        };
        request
            .replacement_state
            .record_replacement(tool_call_id.clone(), record.clone());
        messages[index] = LlmMessage::Tool {
            tool_call_id: tool_call_id.clone(),
            content: replacement.clone(),
        };
        events.push(tool_result_reference_applied_event(
            request.turn_id,
            request.agent,
            &tool_call_id,
            &record.persisted_relative_path,
            &record.replacement,
            record.original_bytes,
        ));
        total_bytes = total_bytes
            .saturating_sub(original_len)
            .saturating_add(replacement.len());
        stats.replacement_count = stats.replacement_count.saturating_add(1);
        stats.bytes_saved = stats.bytes_saved.saturating_add(saved_bytes);
        replaced.insert(tool_call_id);
    }

    for message in &messages[batch_start..] {
        if let LlmMessage::Tool {
            tool_call_id,
            content,
        } = message
        {
            if request
                .replacement_state
                .replacement_for(tool_call_id)
                .is_none()
                && !is_persisted_output(content)
                && !replaced.contains(tool_call_id)
            {
                request.replacement_state.freeze(tool_call_id.clone());
            }
        }
    }

    let _ = request.session_state;
    Ok(ToolResultBudgetOutcome {
        messages,
        events,
        stats,
    })
}

fn trailing_tool_batch_start(messages: &[LlmMessage]) -> Option<usize> {
    let trailing_tools = messages
        .iter()
        .rev()
        .take_while(|message| matches!(message, LlmMessage::Tool { .. }))
        .count();
    if trailing_tools == 0 {
        None
    } else {
        Some(messages.len().saturating_sub(trailing_tools))
    }
}

fn resolve_session_dir(working_dir: &Path, session_id: &str) -> Result<PathBuf> {
    Ok(astrcode_core::project::project_dir(working_dir)?
        .join("sessions")
        .join(session_id))
}

fn extract_persisted_relative_path(replacement: &str) -> Option<String> {
    replacement.lines().find_map(|line| {
        line.split_once("Full output saved to: ")
            .map(|(_, path)| path.trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEventContext, EventTranslator, StorageEvent, UserMessageOrigin};
    use chrono::Utc;

    use super::*;
    use crate::{
        state::append_and_broadcast,
        turn::{events::user_message_event, test_support::test_session_state},
    };

    #[tokio::test]
    async fn aggregate_budget_replaces_largest_fresh_tool_results_and_reapplies_durable_decisions()
    {
        let session_state = test_session_state();
        let tempdir = tempfile::tempdir().expect("tempdir should exist");
        let agent = AgentEventContext::default();
        let mut translator = EventTranslator::new(session_state.current_phase().expect("phase"));
        let replacement = "<persisted-output>\nOutput too large (999 bytes). Full output saved \
                           to: tool-results/call-1.txt\n</persisted-output>";
        append_and_broadcast(
            &session_state,
            &StorageEvent {
                turn_id: Some("turn-prev".to_string()),
                agent: agent.clone(),
                payload: StorageEventPayload::ToolResultReferenceApplied {
                    tool_call_id: "call-1".to_string(),
                    persisted_relative_path: "tool-results/call-1.txt".to_string(),
                    replacement: replacement.to_string(),
                    original_bytes: 999,
                },
            },
            &mut translator,
        )
        .await
        .expect("replacement event should append");
        append_and_broadcast(
            &session_state,
            &user_message_event(
                "turn-1",
                &agent,
                "hello".to_string(),
                UserMessageOrigin::User,
                Utc::now(),
            ),
            &mut translator,
        )
        .await
        .expect("user event should append");

        let mut state = ToolResultReplacementState::seed(&session_state).expect("seed");
        let outcome = apply_tool_result_budget(ApplyToolResultBudgetRequest {
            messages: &[
                LlmMessage::User {
                    content: "hello".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Tool {
                    tool_call_id: "call-1".to_string(),
                    content: "inline should be replaced from durable state".to_string(),
                },
                LlmMessage::Tool {
                    tool_call_id: "call-2".to_string(),
                    content: "x".repeat(2_000),
                },
            ],
            session_id: "session-test",
            working_dir: tempdir.path(),
            session_state: &session_state,
            replacement_state: &mut state,
            aggregate_budget_bytes: 512,
            turn_id: "turn-1",
            agent: &agent,
        })
        .expect("budget application should succeed");

        assert!(matches!(
            &outcome.messages[1],
            LlmMessage::Tool { content, .. } if content == replacement
        ));
        assert!(matches!(
            &outcome.messages[2],
            LlmMessage::Tool { content, .. } if is_persisted_output(content)
        ));
        assert_eq!(outcome.stats.reapply_count, 1);
        assert_eq!(outcome.stats.replacement_count, 1);
        assert_eq!(outcome.stats.over_budget_message_count, 1);
        assert!(outcome.events.iter().any(|event| matches!(
            &event.payload,
            StorageEventPayload::ToolResultReferenceApplied { tool_call_id, .. } if tool_call_id == "call-2"
        )));
    }
}
