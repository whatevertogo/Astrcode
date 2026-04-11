//! # Agent 状态投影
//!
//! 从事件流（`StorageEvent` 序列）中推导出 Agent 的当前状态。
//!
//! ## 核心设计
//!
//! - **纯函数**: `project()` 不产生任何副作用，相同输入总是产生相同输出
//! - **增量应用**: `AgentStateProjector` 支持逐个事件应用，也支持批量处理
//! - **消息重建**: 将 delta 事件（`AssistantDelta`）聚合为完整消息（`AssistantFinal`）
//! - **上下文压缩**: 处理 `CompactApplied` 事件，替换旧消息为摘要
//!
//! ## 为什么需要投影？
//!
//! 事件日志是 append-only 的，但前端和运行时需要知道「当前状态」。
//! 投影器从完整的事件历史中重建出：
//! - 当前消息历史（用于下次 LLM 请求）
//! - 当前阶段（用于 UI 状态指示器）
//! - Turn 计数（用于统计和监控）

use std::path::PathBuf;

use crate::{
    InvocationKind, LlmMessage, Phase, ReasoningContent, SubRunStorageMode, ToolCallRequest,
    UserMessageOrigin,
    event::{StorageEvent, StorageEventPayload},
    format_compact_summary, split_assistant_content,
};

/// Agent 的当前状态快照。
///
/// 由事件流投影而来，包含完整的消息历史和当前阶段。
/// 用于在 turn 之间保持上下文，以及断线重连后恢复状态。
#[derive(Debug, Clone)]
pub struct AgentState {
    /// 会话 ID
    pub session_id: String,
    /// 工作目录
    pub working_dir: PathBuf,
    /// 消息历史（用于下次 LLM 请求的上下文）
    pub messages: Vec<LlmMessage>,
    /// 当前执行阶段
    pub phase: Phase,
    /// 已完成的 turn 数量
    pub turn_count: usize,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            working_dir: PathBuf::new(),
            messages: Vec::new(),
            phase: Phase::Idle,
            turn_count: 0,
        }
    }
}

/// Agent 状态投影器。
///
/// 维护投影过程中的中间状态（如正在累积的 assistant 内容），
/// 支持增量应用单个事件或批量处理事件序列。
///
/// ## 中间状态说明
///
/// - `pending_content`: 正在累积的 assistant 可见文本（来自 `AssistantFinal`）
/// - `pending_reasoning`: 正在累积的推理内容
/// - `pending_tool_calls`: 当前 assistant 消息关联的工具调用列表
///
/// 这些 pending 字段在遇到下一个 User/ToolResult/TurnDone 事件时
/// 通过 `flush_pending_assistant()` 刷入消息历史。
#[derive(Debug, Clone, Default)]
pub struct AgentStateProjector {
    /// 当前已投影的状态
    state: AgentState,
    /// 等待刷入的 assistant 可见文本
    pending_content: Option<String>,
    /// 等待刷入的推理内容
    pending_reasoning: Option<ReasoningContent>,
    /// 等待刷入的工具调用列表
    pending_tool_calls: Vec<ToolCallRequest>,
}

impl AgentStateProjector {
    pub fn from_events(events: &[StorageEvent]) -> Self {
        let mut projector = Self::default();
        for event in events {
            projector.apply(event);
        }
        projector
    }

    pub fn apply(&mut self, event: &StorageEvent) {
        if !should_project_into_session_state(event) {
            return;
        }

        match &event.payload {
            StorageEventPayload::SessionStart {
                session_id,
                working_dir,
                ..
            } => {
                self.state.session_id = session_id.clone();
                self.state.working_dir = PathBuf::from(working_dir);
            },

            StorageEventPayload::UserMessage {
                content, origin, ..
            } => {
                self.flush_pending_assistant();
                if !matches!(origin, UserMessageOrigin::ReactivationPrompt) {
                    self.state.messages.push(LlmMessage::User {
                        content: content.clone(),
                        origin: *origin,
                    });
                }
                self.state.phase = Phase::Thinking;
            },

            StorageEventPayload::AssistantFinal {
                content,
                reasoning_content,
                reasoning_signature,
                ..
            } => {
                self.flush_pending_assistant();
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                self.pending_content = Some(parts.visible_content);
                self.pending_reasoning = parts.reasoning_content.map(|content| ReasoningContent {
                    content,
                    signature: reasoning_signature.clone(),
                });
            },

            StorageEventPayload::ToolCall {
                tool_call_id,
                tool_name,
                args,
                ..
            } => {
                self.pending_tool_calls.push(ToolCallRequest {
                    id: tool_call_id.clone(),
                    name: tool_name.clone(),
                    args: args.clone(),
                });
            },

            StorageEventPayload::ToolResult {
                tool_call_id,
                tool_name,
                output,
                success,
                error,
                metadata,
                duration_ms,
                ..
            } => {
                self.flush_pending_assistant();
                let result = crate::ToolExecutionResult {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    ok: *success,
                    output: output.clone(),
                    error: error.clone(),
                    metadata: metadata.clone(),
                    duration_ms: *duration_ms,
                    truncated: false,
                };
                self.state.messages.push(LlmMessage::Tool {
                    tool_call_id: tool_call_id.clone(),
                    content: result.model_content(),
                });
            },

            StorageEventPayload::CompactApplied {
                summary,
                preserved_recent_turns,
                messages_removed,
                ..
            } => {
                self.flush_pending_assistant();
                self.apply_compaction(
                    summary,
                    *preserved_recent_turns as usize,
                    *messages_removed as usize,
                );
            },

            StorageEventPayload::TurnDone { .. } => {
                self.flush_pending_assistant();
                self.state.phase = Phase::Idle;
                self.state.turn_count += 1;
            },

            StorageEventPayload::AssistantDelta { .. }
            | StorageEventPayload::ToolCallDelta { .. }
            | StorageEventPayload::PromptMetrics { .. }
            | StorageEventPayload::ThinkingDelta { .. }
            | StorageEventPayload::SubRunStarted { .. }
            | StorageEventPayload::SubRunFinished { .. }
            | StorageEventPayload::ChildSessionNotification { .. }
            | StorageEventPayload::AgentMailboxQueued { .. }
            | StorageEventPayload::AgentMailboxBatchStarted { .. }
            | StorageEventPayload::AgentMailboxBatchAcked { .. }
            | StorageEventPayload::AgentMailboxDiscarded { .. }
            | StorageEventPayload::Error { .. } => {},
        }
    }

    pub fn snapshot(&self) -> AgentState {
        let mut clone = self.clone();
        clone.flush_pending_assistant();
        clone.state
    }

    fn flush_pending_assistant(&mut self) {
        if self.pending_content.is_some() || !self.pending_tool_calls.is_empty() {
            let content = self.pending_content.take().unwrap_or_default();
            self.state.messages.push(LlmMessage::Assistant {
                content,
                tool_calls: std::mem::take(&mut self.pending_tool_calls),
                reasoning: self.pending_reasoning.take(),
            });
        }
    }

    fn apply_compaction(
        &mut self,
        summary: &str,
        preserved_recent_turns: usize,
        messages_removed: usize,
    ) {
        // 优先使用事件里记录的 removed 数量精确回放当前阶段的 prefix compaction。
        // 若读取旧事件或异常值，再回退到 preserved_recent_turns 的旧逻辑，保证兼容历史日志。
        let removed = if messages_removed > 0 && messages_removed <= self.state.messages.len() {
            messages_removed
        } else {
            recent_turn_start_index(&self.state.messages, preserved_recent_turns)
                .unwrap_or(self.state.messages.len())
        };
        if removed == 0 {
            return;
        }

        let preserved = self.state.messages.split_off(removed);
        self.state.messages = vec![LlmMessage::User {
            content: format_compact_summary(summary),
            origin: UserMessageOrigin::CompactSummary,
        }];
        self.state.messages.extend(preserved);
    }
}

fn should_project_into_session_state(event: &StorageEvent) -> bool {
    match event.agent_context() {
        None => true,
        Some(agent) => {
            if agent.invocation_kind != Some(InvocationKind::SubRun) {
                return true;
            }

            matches!(
                agent.storage_mode,
                Some(SubRunStorageMode::IndependentSession)
            )
        },
    }
}

fn recent_turn_start_index(
    messages: &[LlmMessage],
    preserved_recent_turns: usize,
) -> Option<usize> {
    let mut seen_turns = 0usize;
    let mut last_index = None;

    for (index, message) in messages.iter().enumerate().rev() {
        if matches!(
            message,
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            }
        ) {
            seen_turns += 1;
            last_index = Some(index);
            if seen_turns >= preserved_recent_turns {
                break;
            }
        }
    }

    last_index
}

/// Pure function: project an event sequence into an AgentState.
/// No IO, no side effects.
pub fn project(events: &[StorageEvent]) -> AgentState {
    AgentStateProjector::from_events(events).snapshot()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{AgentEventContext, StorageEvent, StorageEventPayload};

    fn ts() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    fn root_agent() -> AgentEventContext {
        AgentEventContext::default()
    }

    fn sub_run_agent() -> AgentEventContext {
        AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "explore",
            "subrun-1",
            crate::SubRunStorageMode::IndependentSession,
            None,
        )
    }

    fn event(
        turn_id: Option<&str>,
        agent: AgentEventContext,
        payload: StorageEventPayload,
    ) -> StorageEvent {
        StorageEvent {
            turn_id: turn_id.map(str::to_string),
            agent,
            payload,
        }
    }

    fn session_start(session_id: &str, working_dir: &str) -> StorageEvent {
        event(
            None,
            root_agent(),
            StorageEventPayload::SessionStart {
                session_id: session_id.into(),
                timestamp: ts(),
                working_dir: working_dir.into(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        )
    }

    fn user_message(
        turn_id: Option<&str>,
        agent: AgentEventContext,
        content: &str,
        origin: UserMessageOrigin,
    ) -> StorageEvent {
        event(
            turn_id,
            agent,
            StorageEventPayload::UserMessage {
                content: content.into(),
                origin,
                timestamp: ts(),
            },
        )
    }

    fn assistant_final(
        turn_id: Option<&str>,
        agent: AgentEventContext,
        content: &str,
        reasoning_content: Option<&str>,
    ) -> StorageEvent {
        event(
            turn_id,
            agent,
            StorageEventPayload::AssistantFinal {
                content: content.into(),
                reasoning_content: reasoning_content.map(str::to_string),
                reasoning_signature: None,
                timestamp: None,
            },
        )
    }

    fn turn_done(turn_id: Option<&str>, agent: AgentEventContext, reason: &str) -> StorageEvent {
        event(
            turn_id,
            agent,
            StorageEventPayload::TurnDone {
                timestamp: ts(),
                reason: Some(reason.into()),
            },
        )
    }

    #[test]
    fn empty_events_produce_default_state() {
        let state = project(&[]);
        assert_eq!(state.session_id, "");
        assert!(state.messages.is_empty());
        assert_eq!(state.phase, Phase::Idle);
        assert_eq!(state.turn_count, 0);
    }

    #[test]
    fn session_start_and_user_message() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(None, root_agent(), "hello", UserMessageOrigin::User),
        ];
        let state = project(&events);
        assert_eq!(state.session_id, "s1");
        assert_eq!(state.working_dir, PathBuf::from("/tmp"));
        assert_eq!(state.messages.len(), 1);
        assert!(
            matches!(&state.messages[0], LlmMessage::User { content, .. } if content == "hello")
        );
        assert_eq!(state.phase, Phase::Thinking);
    }

    #[test]
    fn reactivation_prompt_does_not_pollute_projected_messages() {
        let state = project(&[
            session_start("s1", "/tmp"),
            user_message(
                Some("turn-reactivate"),
                root_agent(),
                "# Child Session Delivery",
                UserMessageOrigin::ReactivationPrompt,
            ),
        ]);

        assert!(state.messages.is_empty());
        assert_eq!(state.phase, Phase::Thinking);
    }

    #[test]
    fn turn_done_sets_idle_and_increments_count() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(None, root_agent(), "hi", UserMessageOrigin::User),
            assistant_final(None, root_agent(), "hello!", None),
            turn_done(None, root_agent(), "completed"),
        ];
        let state = project(&events);
        assert_eq!(state.phase, Phase::Idle);
        assert_eq!(state.turn_count, 1);
        assert_eq!(state.messages.len(), 2); // User + Assistant
    }

    #[test]
    fn sub_run_events_do_not_pollute_parent_projected_messages_or_turn_count() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(
                Some("turn-root"),
                root_agent(),
                "root task",
                UserMessageOrigin::User,
            ),
            assistant_final(Some("turn-root"), root_agent(), "root answer", None),
            turn_done(Some("turn-root"), root_agent(), "completed"),
            user_message(
                Some("turn-child"),
                sub_run_agent(),
                "child task",
                UserMessageOrigin::User,
            ),
            assistant_final(Some("turn-child"), sub_run_agent(), "child answer", None),
            turn_done(Some("turn-child"), sub_run_agent(), "completed"),
        ];

        let state = project(&events);

        assert_eq!(
            state.turn_count, 1,
            "sub-run turn must not increment parent turn count"
        );
        assert_eq!(state.phase, Phase::Idle);
        assert_eq!(
            state.messages.len(),
            2,
            "sub-run messages must stay out of parent context"
        );
        assert!(matches!(
            &state.messages[0],
            LlmMessage::User { content, .. } if content == "root task"
        ));
        assert!(matches!(
            &state.messages[1],
            LlmMessage::Assistant { content, .. } if content == "root answer"
        ));
    }

    #[test]
    fn independent_sub_run_events_still_project_into_child_session_state() {
        let child_agent = AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "explore",
            "subrun-independent",
            crate::SubRunStorageMode::IndependentSession,
            Some("session-child".to_string()),
        );
        let events = vec![
            event(
                None,
                root_agent(),
                StorageEventPayload::SessionStart {
                    session_id: "session-child".into(),
                    timestamp: ts(),
                    working_dir: "/tmp".into(),
                    parent_session_id: Some("session-parent".into()),
                    parent_storage_seq: Some(12),
                },
            ),
            user_message(
                Some("turn-child"),
                child_agent.clone(),
                "child task",
                UserMessageOrigin::User,
            ),
            assistant_final(
                Some("turn-child"),
                child_agent.clone(),
                "child answer",
                None,
            ),
            turn_done(Some("turn-child"), child_agent, "completed"),
        ];

        let state = project(&events);

        assert_eq!(state.turn_count, 1);
        assert_eq!(state.messages.len(), 2);
        assert!(matches!(
            &state.messages[0],
            LlmMessage::User { content, .. } if content == "child task"
        ));
        assert!(matches!(
            &state.messages[1],
            LlmMessage::Assistant { content, .. } if content == "child answer"
        ));
    }

    #[test]
    fn multi_turn_with_tool_calls_rebuilds_correctly() {
        let events = vec![
            session_start("s1", "/tmp"),
            // Turn 1: user → assistant with tool call → tool result → final answer
            user_message(None, root_agent(), "list files", UserMessageOrigin::User),
            assistant_final(None, root_agent(), "", None),
            event(
                None,
                root_agent(),
                StorageEventPayload::ToolCall {
                    tool_call_id: "tc1".into(),
                    tool_name: "listDir".into(),
                    args: json!({"path": "."}),
                },
            ),
            event(
                None,
                root_agent(),
                StorageEventPayload::ToolResult {
                    tool_call_id: "tc1".into(),
                    tool_name: "listDir".into(),
                    output: "file1.txt\nfile2.txt".into(),
                    success: true,
                    error: None,
                    metadata: None,
                    duration_ms: 10,
                },
            ),
            assistant_final(None, root_agent(), "Here are the files", None),
            turn_done(None, root_agent(), "completed"),
            // Turn 2: simple user → assistant
            user_message(None, root_agent(), "thanks", UserMessageOrigin::User),
            assistant_final(None, root_agent(), "You're welcome!", None),
            turn_done(None, root_agent(), "completed"),
        ];
        let state = project(&events);

        assert_eq!(state.turn_count, 2);
        assert_eq!(state.phase, Phase::Idle);

        // Turn 1: User, Assistant(empty + tool_calls), Tool, Assistant(final)
        // Turn 2: User, Assistant
        // Total: 6 messages
        assert_eq!(state.messages.len(), 6);

        // First assistant should have one tool_call
        match &state.messages[1] {
            LlmMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                assert_eq!(content, "");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "listDir");
            },
            other => panic!("expected Assistant, got {:?}", other),
        }

        // Tool result
        match &state.messages[2] {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => {
                assert_eq!(tool_call_id, "tc1");
                assert!(content.contains("file1.txt"));
            },
            other => panic!("expected Tool, got {:?}", other),
        }
    }

    #[test]
    fn assistant_delta_and_error_are_ignored() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(None, root_agent(), "hi", UserMessageOrigin::User),
            event(
                None,
                root_agent(),
                StorageEventPayload::AssistantDelta {
                    token: "hel".into(),
                },
            ),
            event(
                None,
                root_agent(),
                StorageEventPayload::AssistantDelta { token: "lo".into() },
            ),
            assistant_final(None, root_agent(), "hello", None),
            event(
                None,
                root_agent(),
                StorageEventPayload::Error {
                    message: "some error".into(),
                    timestamp: Some(ts()),
                },
            ),
            turn_done(None, root_agent(), "completed"),
        ];
        let state = project(&events);
        assert_eq!(state.messages.len(), 2); // User + Assistant only
        assert_eq!(state.turn_count, 1);
    }

    #[test]
    fn tool_messages_require_synthetic_assistant_when_content_is_empty() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(None, root_agent(), "run tool", UserMessageOrigin::User),
            event(
                None,
                root_agent(),
                StorageEventPayload::ToolCall {
                    tool_call_id: "tc1".into(),
                    tool_name: "listDir".into(),
                    args: json!({"path": "."}),
                },
            ),
            event(
                None,
                root_agent(),
                StorageEventPayload::ToolResult {
                    tool_call_id: "tc1".into(),
                    tool_name: "listDir".into(),
                    output: "[]".into(),
                    success: true,
                    error: None,
                    metadata: None,
                    duration_ms: 2,
                },
            ),
            turn_done(None, root_agent(), "completed"),
        ];

        let state = project(&events);
        assert_eq!(state.messages.len(), 3, "expected user + assistant + tool");

        match &state.messages[1] {
            LlmMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                assert_eq!(content, "");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].id, "tc1");
            },
            other => panic!("expected assistant before tool message, got {:?}", other),
        }

        assert!(
            matches!(&state.messages[2], LlmMessage::Tool { tool_call_id, .. } if tool_call_id == "tc1")
        );
    }

    #[test]
    fn incremental_projector_matches_batch_projection() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(None, root_agent(), "hello", UserMessageOrigin::User),
            event(
                None,
                root_agent(),
                StorageEventPayload::AssistantFinal {
                    content: "hi".into(),
                    reasoning_content: Some("thinking".into()),
                    reasoning_signature: Some("sig".into()),
                    timestamp: None,
                },
            ),
            turn_done(None, root_agent(), "completed"),
        ];

        let batch = project(&events);
        let mut projector = AgentStateProjector::default();
        for event in &events {
            projector.apply(event);
        }

        let incremental = projector.snapshot();
        assert_eq!(incremental.session_id, batch.session_id);
        assert_eq!(incremental.working_dir, batch.working_dir);
        assert_eq!(incremental.phase, batch.phase);
        assert_eq!(incremental.turn_count, batch.turn_count);
        assert_eq!(incremental.messages.len(), batch.messages.len());
    }

    #[test]
    fn compact_applied_replaces_old_prefix_with_a_compact_summary_message() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(
                Some("turn-1"),
                root_agent(),
                "first",
                UserMessageOrigin::User,
            ),
            assistant_final(Some("turn-1"), root_agent(), "first-answer", None),
            turn_done(Some("turn-1"), root_agent(), "completed"),
            user_message(
                Some("turn-2"),
                root_agent(),
                "second",
                UserMessageOrigin::User,
            ),
            assistant_final(Some("turn-2"), root_agent(), "second-answer", None),
            turn_done(Some("turn-2"), root_agent(), "completed"),
            event(
                None,
                root_agent(),
                StorageEventPayload::CompactApplied {
                    trigger: crate::event::CompactTrigger::Manual,
                    summary: "condensed work".into(),
                    preserved_recent_turns: 1,
                    pre_tokens: 400,
                    post_tokens_estimate: 120,
                    messages_removed: 2,
                    tokens_freed: 280,
                    timestamp: ts(),
                },
            ),
        ];

        let state = project(&events);

        assert_eq!(state.messages.len(), 3);
        assert!(matches!(
            &state.messages[0],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::CompactSummary,
            } if content.contains("condensed work")
        ));
        assert!(matches!(
            &state.messages[1],
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::User,
            } if content == "second"
        ));
        assert!(matches!(
            &state.messages[2],
            LlmMessage::Assistant { content, .. } if content == "second-answer"
        ));
    }
}
