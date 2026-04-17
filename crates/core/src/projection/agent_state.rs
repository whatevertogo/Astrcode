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

use chrono::{DateTime, Utc};

use crate::{
    InvocationKind, LlmMessage, Phase, ReasoningContent, ToolCallRequest, UserMessageOrigin,
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
    /// 最后一条 assistant 消息的时间戳。
    /// "会话输出静默时间"：从最后一次 assistant 输出算起，
    /// 不是"最后任意事件时间"。用于微压缩时间触发判断。
    pub last_assistant_at: Option<DateTime<Utc>>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            working_dir: PathBuf::new(),
            messages: Vec::new(),
            phase: Phase::Idle,
            turn_count: 0,
            last_assistant_at: None,
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
        if !self.should_project(event) {
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
                if !matches!(
                    origin,
                    UserMessageOrigin::ReactivationPrompt
                        | UserMessageOrigin::AutoContinueNudge
                        | UserMessageOrigin::ContinuationPrompt
                ) {
                    self.state.messages.push(LlmMessage::User {
                        content: content.clone(),
                        origin: *origin,
                    });
                }
                if matches!(origin, UserMessageOrigin::User) {
                    self.state.phase = Phase::Thinking;
                }
            },

            StorageEventPayload::AssistantFinal {
                content,
                reasoning_content,
                reasoning_signature,
                timestamp,
            } => {
                self.flush_pending_assistant();
                let parts = split_assistant_content(content, reasoning_content.as_deref());
                self.pending_content = Some(parts.visible_content);
                self.pending_reasoning = parts.reasoning_content.map(|content| ReasoningContent {
                    content,
                    signature: reasoning_signature.clone(),
                });
                // 只在 timestamp 为 Some 时更新，None 时保留旧值
                if let Some(ts) = timestamp {
                    self.state.last_assistant_at = Some(*ts);
                }
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
                child_ref,
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
                    child_ref: child_ref.clone(),
                    duration_ms: *duration_ms,
                    truncated: false,
                };
                self.state.messages.push(LlmMessage::Tool {
                    tool_call_id: tool_call_id.clone(),
                    content: result.model_content(),
                });
            },
            StorageEventPayload::ToolResultReferenceApplied {
                tool_call_id,
                replacement,
                ..
            } => {
                if let Some(LlmMessage::Tool { content, .. }) = self
                    .state
                    .messages
                    .iter_mut()
                    .rev()
                    .find(|message| matches!(message, LlmMessage::Tool { tool_call_id: current, .. } if current == tool_call_id))
                {
                    *content = replacement.clone();
                }
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
            | StorageEventPayload::AgentCollaborationFact { .. }
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

    /// 判断事件是否应投影到当前会话状态。
    ///
    /// - 非 SubRun 事件：始终投影
    /// - SubRun：仅在事件明确属于当前独立子会话时投影
    fn should_project(&self, event: &StorageEvent) -> bool {
        match event.agent_context() {
            None => true,
            Some(agent) => {
                agent.invocation_kind != Some(InvocationKind::SubRun)
                    || agent.belongs_to_child_session(&self.state.session_id)
            },
        }
    }
}

/// 从消息列表末尾向前扫描，找到第 N 个 User-origin 消息的位置。
/// 用途：定义"保留最近 N 轮"的裁剪边界，User-origin 消息视为 turn 起点。
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
    use crate::{
        AgentEventContext, CompactAppliedMeta, CompactMode, CompactTrigger, StorageEvent,
        StorageEventPayload, SubRunStorageMode,
    };

    fn ts() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    fn root_agent() -> AgentEventContext {
        AgentEventContext::default()
    }

    fn child_agent(session_id: &str) -> AgentEventContext {
        AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "explore",
            "subrun-1",
            None,
            SubRunStorageMode::IndependentSession,
            Some(session_id.into()),
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

    fn child_session_start(
        session_id: &str,
        parent_session_id: &str,
        parent_storage_seq: u64,
        working_dir: &str,
    ) -> StorageEvent {
        event(
            None,
            root_agent(),
            StorageEventPayload::SessionStart {
                session_id: session_id.into(),
                timestamp: ts(),
                working_dir: working_dir.into(),
                parent_session_id: Some(parent_session_id.into()),
                parent_storage_seq: Some(parent_storage_seq),
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

    fn tool_call(
        turn_id: Option<&str>,
        agent: AgentEventContext,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> StorageEvent {
        event(
            turn_id,
            agent,
            StorageEventPayload::ToolCall {
                tool_call_id: tool_call_id.into(),
                tool_name: tool_name.into(),
                args,
            },
        )
    }

    fn tool_result(
        turn_id: Option<&str>,
        agent: AgentEventContext,
        tool_call_id: &str,
        tool_name: &str,
        output: &str,
        duration_ms: u64,
    ) -> StorageEvent {
        event(
            turn_id,
            agent,
            StorageEventPayload::ToolResult {
                tool_call_id: tool_call_id.into(),
                tool_name: tool_name.into(),
                output: output.into(),
                success: true,
                error: None,
                metadata: None,
                child_ref: None,
                duration_ms,
            },
        )
    }

    fn tool_result_reference_applied(
        turn_id: Option<&str>,
        agent: AgentEventContext,
        tool_call_id: &str,
        replacement: &str,
    ) -> StorageEvent {
        event(
            turn_id,
            agent,
            StorageEventPayload::ToolResultReferenceApplied {
                tool_call_id: tool_call_id.into(),
                persisted_output: crate::PersistedToolOutput {
                    storage_kind: "toolResult".to_string(),
                    absolute_path: "~/.astrcode/tool-results/sample.txt".to_string(),
                    relative_path: "tool-results/sample.txt".to_string(),
                    total_bytes: 120,
                    preview_text: "preview".to_string(),
                    preview_bytes: 7,
                },
                replacement: replacement.to_string(),
                original_bytes: 120,
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

    fn compact_applied(
        summary: &str,
        preserved_recent_turns: u32,
        messages_removed: u32,
    ) -> StorageEvent {
        event(
            None,
            root_agent(),
            StorageEventPayload::CompactApplied {
                trigger: CompactTrigger::Manual,
                summary: summary.into(),
                meta: CompactAppliedMeta {
                    mode: CompactMode::Full,
                    instructions_present: false,
                    fallback_used: false,
                    retry_count: 0,
                    input_units: 3,
                    output_summary_chars: 15,
                },
                preserved_recent_turns,
                pre_tokens: 400,
                post_tokens_estimate: 120,
                messages_removed,
                tokens_freed: 280,
                timestamp: ts(),
            },
        )
    }

    fn assert_user_message(
        message: &LlmMessage,
        expected_content: &str,
        origin: UserMessageOrigin,
    ) {
        assert!(
            matches!(
                message,
                LlmMessage::User {
                    content,
                    origin: actual_origin,
                } if content == expected_content && *actual_origin == origin
            ),
            "expected user message `{expected_content}` with origin {origin:?}, got {message:?}"
        );
    }

    fn assert_assistant_message(message: &LlmMessage, expected_content: &str) {
        assert!(
            matches!(
                message,
                LlmMessage::Assistant { content, .. } if content == expected_content
            ),
            "expected assistant message `{expected_content}`, got {message:?}"
        );
    }

    fn assert_compact_summary_message(message: &LlmMessage, expected_summary: &str) {
        assert!(
            matches!(
                message,
                LlmMessage::User {
                    content,
                    origin: UserMessageOrigin::CompactSummary,
                } if content.contains(expected_summary)
            ),
            "expected compact summary containing `{expected_summary}`, got {message:?}"
        );
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
        assert_user_message(&state.messages[0], "hello", UserMessageOrigin::User);
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
        assert_eq!(state.phase, Phase::Idle);
    }

    #[test]
    fn internal_continuation_prompts_do_not_pollute_projected_messages() {
        let state = project(&[
            session_start("s1", "/tmp"),
            user_message(
                Some("turn-internal"),
                root_agent(),
                "请继续。",
                UserMessageOrigin::AutoContinueNudge,
            ),
            user_message(
                Some("turn-internal"),
                root_agent(),
                "从上次截断处继续。",
                UserMessageOrigin::ContinuationPrompt,
            ),
        ]);

        assert!(state.messages.is_empty());
        assert_eq!(state.phase, Phase::Idle);
    }

    #[test]
    fn internal_compact_summary_message_does_not_start_a_new_turn_phase() {
        let state = project(&[
            session_start("s1", "/tmp"),
            user_message(
                Some("turn-compact"),
                root_agent(),
                &format_compact_summary("summary"),
                UserMessageOrigin::CompactSummary,
            ),
        ]);

        assert_eq!(state.messages.len(), 1);
        assert_user_message(
            &state.messages[0],
            &format_compact_summary("summary"),
            UserMessageOrigin::CompactSummary,
        );
        assert_eq!(state.phase, Phase::Idle);
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
                child_agent("session-child"),
                "child task",
                UserMessageOrigin::User,
            ),
            assistant_final(
                Some("turn-child"),
                child_agent("session-child"),
                "child answer",
                None,
            ),
            turn_done(
                Some("turn-child"),
                child_agent("session-child"),
                "completed",
            ),
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
        assert_user_message(&state.messages[0], "root task", UserMessageOrigin::User);
        assert_assistant_message(&state.messages[1], "root answer");
    }

    #[test]
    fn independent_sub_run_events_still_project_into_child_session_state() {
        let child_agent = child_agent("session-child");
        let events = vec![
            child_session_start("session-child", "session-parent", 12, "/tmp"),
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
        assert_user_message(&state.messages[0], "child task", UserMessageOrigin::User);
        assert_assistant_message(&state.messages[1], "child answer");
    }

    #[test]
    fn multi_turn_with_tool_calls_rebuilds_correctly() {
        let events = vec![
            session_start("s1", "/tmp"),
            // Turn 1: user → assistant with tool call → tool result → final answer
            user_message(None, root_agent(), "list files", UserMessageOrigin::User),
            assistant_final(None, root_agent(), "", None),
            tool_call(None, root_agent(), "tc1", "listDir", json!({"path": "."})),
            tool_result(
                None,
                root_agent(),
                "tc1",
                "listDir",
                "file1.txt\nfile2.txt",
                10,
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
            tool_call(None, root_agent(), "tc1", "listDir", json!({"path": "."})),
            tool_result(None, root_agent(), "tc1", "listDir", "[]", 2),
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
    fn tool_result_reference_applied_rewrites_projected_tool_content() {
        let events = vec![
            session_start("s1", "/tmp"),
            user_message(None, root_agent(), "run tool", UserMessageOrigin::User),
            tool_call(
                None,
                root_agent(),
                "tc1",
                "readFile",
                json!({"path": "src/lib.rs"}),
            ),
            tool_result(None, root_agent(), "tc1", "readFile", "inline output", 2),
            tool_result_reference_applied(
                None,
                root_agent(),
                "tc1",
                "<persisted-output>\nLarge tool output was saved to a file instead of being \
                 inlined.\nPath: ~/.astrcode/tool-results/sample.txt\nBytes: 120\nRead the file \
                 with \
                 `readFile`.\nIf you only need a section, read a smaller chunk instead of the \
                 whole file.\nStart from the first chunk when you do not yet know the right \
                 section.\nSuggested first read: { path: \
                 \"~/.astrcode/tool-results/sample.txt\", charOffset: 0, maxChars: 20000 }\n\
                 </persisted-output>",
            ),
            turn_done(None, root_agent(), "completed"),
        ];

        let state = project(&events);

        assert!(matches!(
            &state.messages[2],
            LlmMessage::Tool { content, .. } if content.contains("~/.astrcode/tool-results/sample.txt")
        ));
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
            compact_applied("condensed work", 1, 2),
        ];

        let state = project(&events);

        assert_eq!(state.messages.len(), 3);
        assert_compact_summary_message(&state.messages[0], "condensed work");
        assert_user_message(&state.messages[1], "second", UserMessageOrigin::User);
        assert_assistant_message(&state.messages[2], "second-answer");
    }
}
