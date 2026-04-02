//! # 会话回放 (Session Replay)
//!
//! 实现 `SessionReplaySource` trait，为 SSE 客户端提供会话历史回放和实时订阅。
//!
//! ## 回放路径
//!
//! 1. **缓存路径（Cache）**: 优先从内存缓存（`RecentSessionEvents`）读取，快速返回
//! 2. **磁盘回退（DiskFallback）**: 缓存不足时从磁盘 JSONL 文件加载
//!
//! ## 事件转换
//!
//! `convert_events_to_messages` 将存储事件序列转换为面向前端展示的会话消息列表。
//! 关键设计决策：
//! - **reasoning-only 消息合并**: 纯推理消息（无可见内容）如果有后续工具调用，
//!   合并到工具调用消息中；否则丢弃（前端不需要展示空的推理块）
//! - **工具调用聚合**: 同一 step 的多个工具调用聚合到一个消息中
//! - **流式工具输出**: `ToolOutputDelta` 事件聚合为完整的工具结果

use astrcode_core::{
    replay_records, split_assistant_content, AstrError, ToolOutputDelta, ToolOutputStream,
    UserMessageOrigin,
};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use astrcode_core::{StorageEvent, StoredEvent};
use serde_json::{json, Map, Value};

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
    let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
    let mut pending_reasoning_only_assistants: HashMap<String, Option<String>> = HashMap::new();

    for stored in events {
        match &stored.event {
            StorageEvent::UserMessage {
                turn_id,
                content,
                timestamp,
                origin,
            } => {
                if !matches!(origin, UserMessageOrigin::User) {
                    continue;
                }
                messages.push(SessionMessage::User {
                    turn_id: turn_id.clone(),
                    content: content.clone(),
                    timestamp: timestamp.to_rfc3339(),
                });
            }
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
            } => pending_tool_calls.push(PendingToolCall {
                turn_id: turn_id.clone(),
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                args: args.clone(),
                output: String::new(),
                segments: Vec::new(),
            }),
            StorageEvent::ToolCallDelta {
                tool_call_id,
                tool_name,
                stream,
                delta,
                ..
            } => {
                if let Some(pending) = pending_tool_calls
                    .iter_mut()
                    .find(|pending| pending.tool_call_id == *tool_call_id)
                {
                    if pending.tool_name.is_empty() && !tool_name.is_empty() {
                        pending.tool_name = tool_name.clone();
                    }
                    pending.push_delta(ToolOutputDelta {
                        tool_call_id: tool_call_id.clone(),
                        tool_name: if tool_name.is_empty() {
                            pending.tool_name.clone()
                        } else {
                            tool_name.clone()
                        },
                        stream: *stream,
                        delta: delta.clone(),
                    });
                }
            }
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
                    .position(|pending| pending.tool_call_id == *tool_call_id)
                {
                    let pending = pending_tool_calls.remove(index);
                    let tool_name = if stored_tool_name.is_empty() {
                        pending.tool_name.clone()
                    } else {
                        stored_tool_name.clone()
                    };
                    let result = astrcode_core::ToolExecutionResult {
                        tool_call_id: tool_call_id.clone(),
                        // ToolCall 事件总是包含工具名，但 ToolResult 事件可能为空
                        // （如旧版格式或异常恢复场景）。此时使用匹配的 pending ToolCall
                        // 中的工具名作为回退，确保前端能正确显示工具卡片。
                        tool_name: tool_name.clone(),
                        ok: *success,
                        output: if pending.output.is_empty() {
                            output.clone()
                        } else {
                            pending.output.clone()
                        },
                        error: error.clone(),
                        metadata: decorate_tool_metadata(
                            &tool_name,
                            &pending.args,
                            metadata.as_ref(),
                            &pending.segments,
                        ),
                        duration_ms: *duration_ms,
                        truncated: false,
                    };
                    messages.push(SessionMessage::ToolCall {
                        turn_id: turn_id.clone().or(pending.turn_id),
                        tool_call_id: tool_call_id.clone(),
                        tool_name: result.tool_name.clone(),
                        args: pending.args,
                        output: (!result.output.is_empty()).then_some(result.output.clone()),
                        error: result.error.clone(),
                        metadata: result.metadata.clone(),
                        ok: Some(*success),
                        duration_ms: Some(*duration_ms),
                    });
                }
            }
            StorageEvent::PromptMetrics { .. } | StorageEvent::CompactApplied { .. } => {}
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
    for pending in pending_tool_calls {
        messages.push(SessionMessage::ToolCall {
            turn_id: pending.turn_id,
            tool_call_id: pending.tool_call_id.clone(),
            tool_name: pending.tool_name.clone(),
            args: pending.args.clone(),
            output: (!pending.output.is_empty()).then_some(pending.output.clone()),
            error: None,
            metadata: decorate_tool_metadata(
                &pending.tool_name,
                &pending.args,
                None,
                &pending.segments,
            ),
            ok: None,
            duration_ms: None,
        });
    }

    messages
}

#[derive(Clone, Debug)]
struct PendingToolCall {
    turn_id: Option<String>,
    tool_call_id: String,
    tool_name: String,
    args: Value,
    output: String,
    segments: Vec<ToolOutputSegment>,
}

impl PendingToolCall {
    fn push_delta(&mut self, delta: ToolOutputDelta) {
        self.output.push_str(&delta.delta);
        match self.segments.last_mut() {
            Some(last) if last.stream == delta.stream => last.text.push_str(&delta.delta),
            _ => self.segments.push(ToolOutputSegment {
                stream: delta.stream,
                text: delta.delta,
            }),
        }
    }
}

#[derive(Clone, Debug)]
struct ToolOutputSegment {
    stream: ToolOutputStream,
    text: String,
}

fn decorate_tool_metadata(
    tool_name: &str,
    args: &Value,
    metadata: Option<&Value>,
    segments: &[ToolOutputSegment],
) -> Option<Value> {
    let mut object = metadata
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);

    if tool_name != "shell" {
        return if object.is_empty() {
            None
        } else {
            Some(Value::Object(object))
        };
    }

    object.insert(
        "display".to_string(),
        build_shell_display_metadata(args, metadata, segments),
    );

    Some(Value::Object(object))
}

fn build_shell_display_metadata(
    args: &Value,
    metadata: Option<&Value>,
    segments: &[ToolOutputSegment],
) -> Value {
    let command = args.get("command").and_then(Value::as_str).or_else(|| {
        metadata
            .and_then(|value| value.get("command"))
            .and_then(Value::as_str)
    });
    let cwd = args.get("cwd").and_then(Value::as_str).or_else(|| {
        metadata
            .and_then(|value| value.get("cwd"))
            .and_then(Value::as_str)
    });
    let shell = args.get("shell").and_then(Value::as_str).or_else(|| {
        metadata
            .and_then(|value| value.get("shell"))
            .and_then(Value::as_str)
    });
    let exit_code = metadata
        .and_then(|value| value.get("exitCode"))
        .and_then(Value::as_i64);

    let mut display = Map::new();
    display.insert("kind".to_string(), json!("terminal"));
    if let Some(command) = command {
        display.insert("command".to_string(), json!(command));
    }
    if let Some(cwd) = cwd {
        display.insert("cwd".to_string(), json!(cwd));
    }
    if let Some(shell) = shell {
        display.insert("shell".to_string(), json!(shell));
    }
    if let Some(exit_code) = exit_code {
        display.insert("exitCode".to_string(), json!(exit_code));
    }
    if !segments.is_empty() {
        display.insert(
            "segments".to_string(),
            Value::Array(
                segments
                    .iter()
                    .map(|segment| {
                        json!({
                            "stream": segment.stream,
                            "text": segment.text,
                        })
                    })
                    .collect(),
            ),
        );
    }

    Value::Object(display)
}

#[cfg(test)]
mod tests {
    use astrcode_core::UserMessageOrigin;
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
                    origin: UserMessageOrigin::User,
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
                    reason: Some("completed".to_string()),
                },
            ),
        ];

        let messages = convert_events_to_messages(&events);

        assert_eq!(messages.len(), 1, "expected only the finished tool row");
        assert!(matches!(messages[0], SessionMessage::ToolCall { .. }));
    }

    #[test]
    fn snapshot_rebuilds_shell_terminal_metadata_from_tool_deltas() {
        let events = vec![
            stored(
                1,
                StorageEvent::ToolCall {
                    turn_id: Some("turn-shell".to_string()),
                    tool_call_id: "tc-shell".to_string(),
                    tool_name: "shell".to_string(),
                    args: json!({"command": "echo ok", "cwd": "/repo"}),
                },
            ),
            stored(
                2,
                StorageEvent::ToolCallDelta {
                    turn_id: Some("turn-shell".to_string()),
                    tool_call_id: "tc-shell".to_string(),
                    tool_name: "shell".to_string(),
                    stream: ToolOutputStream::Stdout,
                    delta: "ok\n".to_string(),
                },
            ),
            stored(
                3,
                StorageEvent::ToolCallDelta {
                    turn_id: Some("turn-shell".to_string()),
                    tool_call_id: "tc-shell".to_string(),
                    tool_name: "shell".to_string(),
                    stream: ToolOutputStream::Stderr,
                    delta: "warn\n".to_string(),
                },
            ),
            stored(
                4,
                StorageEvent::ToolResult {
                    turn_id: Some("turn-shell".to_string()),
                    tool_call_id: "tc-shell".to_string(),
                    tool_name: "shell".to_string(),
                    output: "[stdout]\nok\n\n[stderr]\nwarn\n".to_string(),
                    success: true,
                    error: None,
                    metadata: Some(json!({
                        "command": "echo ok",
                        "cwd": "/repo",
                        "exitCode": 0,
                    })),
                    duration_ms: 9,
                },
            ),
        ];

        let messages = convert_events_to_messages(&events);
        let shell = match &messages[0] {
            SessionMessage::ToolCall {
                tool_name,
                output,
                metadata,
                ..
            } => {
                assert_eq!(tool_name, "shell");
                assert_eq!(output.as_deref(), Some("ok\nwarn\n"));
                metadata.as_ref().expect("shell metadata should exist")
            }
            other => panic!("expected tool call message, got {other:?}"),
        };

        assert_eq!(shell["display"]["kind"], json!("terminal"));
        assert_eq!(shell["display"]["command"], json!("echo ok"));
        assert_eq!(shell["display"]["cwd"], json!("/repo"));
        assert_eq!(shell["display"]["exitCode"], json!(0));
        assert_eq!(shell["display"]["segments"][0]["stream"], json!("stdout"));
        assert_eq!(shell["display"]["segments"][1]["stream"], json!("stderr"));
    }
}
