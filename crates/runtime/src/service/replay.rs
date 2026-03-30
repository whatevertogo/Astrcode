use astrcode_core::{replay_records, split_assistant_content, AstrError};
use async_trait::async_trait;
use chrono::Utc;
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
            None => load_events(&session_id).await.map(|events| {
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

pub(super) fn convert_events_to_messages(events: &[StoredEvent]) -> Vec<SessionMessage> {
    let mut messages = Vec::new();
    let mut pending_tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();

    for stored in events {
        match &stored.event {
            StorageEvent::UserMessage {
                content, timestamp, ..
            } => messages.push(SessionMessage::User {
                content: content.clone(),
                timestamp: timestamp.to_rfc3339(),
            }),
            StorageEvent::AssistantFinal {
                content,
                reasoning_content,
                timestamp,
                ..
            } => {
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                if parts.visible_content.is_empty() && parts.reasoning_content.is_none() {
                    continue;
                }
                messages.push(SessionMessage::Assistant {
                    content: parts.visible_content,
                    timestamp: timestamp
                        .as_ref()
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| Utc::now().to_rfc3339()),
                    reasoning_content: parts.reasoning_content,
                });
            }
            StorageEvent::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => pending_tool_calls.push((tool_call_id.clone(), tool_name.clone(), args.clone())),
            StorageEvent::ToolResult {
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
                    .position(|(pending_id, _, _)| pending_id == tool_call_id)
                {
                    let (_, pending_tool_name, args) = pending_tool_calls.remove(index);
                    let result = astrcode_core::ToolExecutionResult {
                        tool_call_id: tool_call_id.clone(),
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
                        tool_call_id: tool_call_id.clone(),
                        tool_name: result.tool_name.clone(),
                        args,
                        output: Some(result.model_content()),
                        ok: Some(*success),
                        duration_ms: Some(*duration_ms),
                    });
                }
            }
            _ => {}
        }
    }

    for (tool_call_id, tool_name, args) in pending_tool_calls {
        messages.push(SessionMessage::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output: None,
            ok: None,
            duration_ms: None,
        });
    }

    messages
}
