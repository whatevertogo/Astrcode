use astrcode_core::{replay_records, split_assistant_content, AstrError};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use astrcode_core::{StorageEvent, StoredEvent};

use super::session_ops::{load_events, normalize_session_id};
use super::{
    ReplayPath, RuntimeService, ServiceResult, SessionMessage, SessionReplay, SessionReplaySource,
};

#[async_trait]
impl SessionReplaySource for RuntimeService {
    async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay> {
        let session_id = normalize_session_id(session_id);
        let state = self.ensure_session_loaded(&session_id).await?;

        let receiver = state.broadcaster.subscribe();
        let started_at = Instant::now();
        let replay_result = match state
            .recent_records_after(last_event_id)
            .map_err(|error| AstrError::Internal(error.to_string()))?
        {
            Some(history) => Ok((history, ReplayPath::Cache)),
            None => load_events(Arc::clone(&self.session_manager), &session_id)
                .await
                .map(|events| {
                    (
                        replay_records(&events, last_event_id),
                        ReplayPath::DiskFallback,
                    )
                }),
        };
        let elapsed = started_at.elapsed();
        match &replay_result {
            Ok((history, path)) => {
                self.observability
                    .record_sse_catch_up(elapsed, true, path.clone(), history.len());
                if matches!(path, ReplayPath::DiskFallback) {
                    log::warn!(
                        "session '{}' replay used durable fallback and recovered {} events in {}ms",
                        session_id,
                        history.len(),
                        elapsed.as_millis()
                    );
                }
            }
            Err(error) => {
                self.observability
                    .record_sse_catch_up(elapsed, false, ReplayPath::DiskFallback, 0);
                log::error!(
                    "failed to replay session '{}' after {}ms: {}",
                    session_id,
                    elapsed.as_millis(),
                    error
                );
            }
        }
        let (history, _) = replay_result?;
        Ok(SessionReplay { history, receiver })
    }
}

/// 将存储事件序列转换为面向前端展示的会话消息列表。
///
/// ## 关键设计决策
///
/// **reasoning-only 消息合并**: 当 `AssistantFinal` 仅包含推理内容（`reasoning_content`）
/// 而没有可见内容（`content` 为空）时，先缓存到 `pending_reasoning_only_assistants`。
/// 如果紧接着出现 `ToolCall`，说明推理内容是工具调用的前置思考，应合并到工具调用消息中；
/// 如果没有工具调用（如 turn 结束），则丢弃这些纯推理消息——前端不需要展示空的推理块。
pub(super) fn convert_events_to_messages(events: &[StoredEvent]) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    let mut pending_tool_calls: Vec<(Option<String>, String, String, serde_json::Value)> =
        Vec::new();
    let mut pending_reasoning_only_assistants: HashMap<String, Option<String>> = HashMap::new();

    for stored in events {
        match &stored.event {
            StorageEvent::UserMessage {
                turn_id,
                content,
                timestamp,
            } => messages.push(SessionMessage::User {
                turn_id: turn_id.clone(),
                content: content.clone(),
                timestamp: timestamp.to_rfc3339(),
            }),
            StorageEvent::AssistantFinal {
                turn_id,
                content,
                reasoning_content,
                timestamp,
                ..
            } => {
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                if parts.visible_content.is_empty() && parts.reasoning_content.is_none() {
                    continue;
                }
                let timestamp = timestamp
                    .as_ref()
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_else(|| Utc::now().to_rfc3339());
                if parts.visible_content.is_empty() {
                    if let Some(turn_id) = turn_id.clone() {
                        // Tool-planning assistant steps are an internal bridge state; keep only
                        // the latest reasoning per turn so history does not explode into
                        // thinking-only rows before every tool card.
                        pending_reasoning_only_assistants.insert(turn_id, parts.reasoning_content);
                        continue;
                    }
                }
                if let Some(turn_id) = turn_id.as_ref() {
                    pending_reasoning_only_assistants.remove(turn_id);
                }
                messages.push(SessionMessage::Assistant {
                    turn_id: turn_id.clone(),
                    content: parts.visible_content,
                    timestamp,
                    reasoning_content: parts.reasoning_content,
                });
            }
            StorageEvent::ToolCall {
                turn_id,
                tool_call_id,
                tool_name,
                args,
                ..
            } => pending_tool_calls.push((
                turn_id.clone(),
                tool_call_id.clone(),
                tool_name.clone(),
                args.clone(),
            )),
            StorageEvent::ToolResult {
                turn_id,
                tool_call_id,
                tool_name: stored_tool_name,
                output,
                success,
                error,
                metadata,
                duration_ms,
                ..
            } => {
                if let Some(index) = pending_tool_calls
                    .iter()
                    .position(|(_, pending_id, _, _)| pending_id == tool_call_id)
                {
                    let (pending_turn_id, _, pending_tool_name, args) =
                        pending_tool_calls.remove(index);
                    let result = astrcode_core::ToolExecutionResult {
                        tool_call_id: tool_call_id.clone(),
                        // ToolCall 事件总是包含工具名，但 ToolResult 事件可能为空
                        // （如旧版格式或异常恢复场景）。此时使用匹配的 pending ToolCall
                        // 中的工具名作为回退，确保前端能正确显示工具卡片。
                        tool_name: if stored_tool_name.is_empty() {
                            pending_tool_name
                        } else {
                            stored_tool_name.clone()
                        },
                        ok: *success,
                        output: output.clone(),
                        error: error.clone(),
                        metadata: metadata.clone(),
                        duration_ms: *duration_ms,
                        truncated: false,
                    };
                    messages.push(SessionMessage::ToolCall {
                        turn_id: turn_id.clone().or(pending_turn_id),
                        tool_call_id: tool_call_id.clone(),
                        tool_name: result.tool_name.clone(),
                        args,
                        output: (!result.output.is_empty()).then_some(result.output.clone()),
                        error: result.error.clone(),
                        metadata: result.metadata.clone(),
                        ok: Some(*success),
                        duration_ms: Some(*duration_ms),
                    });
                }
            }
            StorageEvent::TurnDone { turn_id, .. } | StorageEvent::Error { turn_id, .. } => {
                if let Some(turn_id) = turn_id {
                    pending_reasoning_only_assistants.remove(turn_id);
                }
            }
            _ => {}
        }
    }

    // 孤立的 pending_tool_calls：工具被调用但未返回结果。
    // 这代表两种场景：(1) 快照发生在 turn 执行中间（正常状态，前端显示 loading）；
    // (2) 进程在工具执行期间崩溃（异常恢复，前端显示无结果的工具卡片）。
    // 无论哪种情况，前端都需要看到这些 ToolCall 以保持 UI 一致性。
    for (turn_id, tool_call_id, tool_name, args) in pending_tool_calls {
        messages.push(SessionMessage::ToolCall {
            turn_id,
            tool_call_id,
            tool_name,
            args,
            output: None,
            error: None,
            metadata: None,
            ok: None,
            duration_ms: None,
        });
    }

    messages
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn stored(storage_seq: u64, event: StorageEvent) -> StoredEvent {
        StoredEvent { storage_seq, event }
    }

    #[test]
    fn snapshot_skips_reasoning_only_assistant_before_tool_rows() {
        let events = vec![
            stored(
                1,
                StorageEvent::UserMessage {
                    turn_id: Some("turn-1".to_string()),
                    content: "调用一下工具给我看看".to_string(),
                    timestamp: Utc.with_ymd_and_hms(2026, 3, 31, 15, 0, 0).unwrap(),
                },
            ),
            stored(
                2,
                StorageEvent::AssistantFinal {
                    turn_id: Some("turn-1".to_string()),
                    content: String::new(),
                    reasoning_content: Some("tool planning".to_string()),
                    reasoning_signature: None,
                    timestamp: Some(Utc.with_ymd_and_hms(2026, 3, 31, 15, 0, 1).unwrap()),
                },
            ),
            stored(
                3,
                StorageEvent::ToolCall {
                    turn_id: Some("turn-1".to_string()),
                    tool_call_id: "tc-1".to_string(),
                    tool_name: "listDir".to_string(),
                    args: json!({"path": "."}),
                },
            ),
            stored(
                4,
                StorageEvent::ToolResult {
                    turn_id: Some("turn-1".to_string()),
                    tool_call_id: "tc-1".to_string(),
                    tool_name: "listDir".to_string(),
                    output: "[]".to_string(),
                    success: true,
                    error: None,
                    metadata: None,
                    duration_ms: 1,
                },
            ),
            stored(
                5,
                StorageEvent::AssistantFinal {
                    turn_id: Some("turn-1".to_string()),
                    content: "目录里有 0 项。".to_string(),
                    reasoning_content: Some("final reasoning".to_string()),
                    reasoning_signature: None,
                    timestamp: Some(Utc.with_ymd_and_hms(2026, 3, 31, 15, 0, 2).unwrap()),
                },
            ),
        ];

        let messages = convert_events_to_messages(&events);

        assert_eq!(messages.len(), 3, "expected user + tool + assistant");
        assert!(matches!(messages[1], SessionMessage::ToolCall { .. }));
        assert!(matches!(
            &messages[2],
            SessionMessage::Assistant { content, .. } if content == "目录里有 0 项。"
        ));
    }

    #[test]
    fn snapshot_drops_reasoning_only_assistant_when_turn_ends_after_tool() {
        let events = vec![
            stored(
                1,
                StorageEvent::AssistantFinal {
                    turn_id: Some("turn-1".to_string()),
                    content: String::new(),
                    reasoning_content: Some("tool planning".to_string()),
                    reasoning_signature: None,
                    timestamp: Some(Utc.with_ymd_and_hms(2026, 3, 31, 15, 0, 1).unwrap()),
                },
            ),
            stored(
                2,
                StorageEvent::ToolCall {
                    turn_id: Some("turn-1".to_string()),
                    tool_call_id: "tc-1".to_string(),
                    tool_name: "listDir".to_string(),
                    args: json!({"path": "."}),
                },
            ),
            stored(
                3,
                StorageEvent::ToolResult {
                    turn_id: Some("turn-1".to_string()),
                    tool_call_id: "tc-1".to_string(),
                    tool_name: "listDir".to_string(),
                    output: "[]".to_string(),
                    success: true,
                    error: None,
                    metadata: None,
                    duration_ms: 1,
                },
            ),
            stored(
                4,
                StorageEvent::TurnDone {
                    turn_id: Some("turn-1".to_string()),
                    timestamp: Utc.with_ymd_and_hms(2026, 3, 31, 15, 0, 2).unwrap(),
                },
            ),
        ];

        let messages = convert_events_to_messages(&events);

        assert_eq!(messages.len(), 1, "expected only the finished tool row");
        assert!(matches!(messages[0], SessionMessage::ToolCall { .. }));
    }
}
