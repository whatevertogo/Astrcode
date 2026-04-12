//! 子 Agent 上下文解析模块。
//!
//! 负责将父会话状态和子 Agent 参数组合成完整的上下文快照，包括：
//! - 任务主体（prompt）和补充上下文（context）
//! - 父会话的 compact summary 继承
//! - 父会话的最近 N 轮对话 tail 继承
//! - 执行谱系索引（跨会话的血缘关系追踪）
//!
//! 设计原则：纯函数无状态，便于测试和复用。
//! 从 runtime-execution/context.rs 迁移，去除对旧 crate 的依赖。

use std::collections::HashMap;

use astrcode_core::{
    AgentEvent, AgentState, ForkMode, LlmMessage, SessionEventRecord, StorageEvent,
    StorageEventPayload, StoredEvent, UserMessageOrigin, parse_compact_summary_message,
};

use crate::execution::prep::AgentExecutionRequest;

// ── 上下文快照解析 ──────────────────────────────────────────

/// 将父会话状态和请求参数组合为子 Agent 的上下文快照。
pub fn resolve_context_snapshot(
    params: &AgentExecutionRequest,
    parent_state: Option<&AgentState>,
    overrides: &astrcode_core::ResolvedSubagentContextOverrides,
) -> ResolvedContextSnapshot {
    let inherited_compact_summary = if overrides.include_compact_summary {
        parent_state.and_then(latest_compact_summary)
    } else {
        None
    };
    let inherited_recent_tail = if overrides.include_recent_tail {
        parent_state
            .map(|state| inherited_recent_tail_lines(state, overrides))
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    ResolvedContextSnapshot {
        task_payload: build_task_payload(params),
        inherited_compact_summary,
        inherited_recent_tail,
    }
}

/// 子 Agent 的上下文快照，包含任务正文和继承的父会话信息。
#[derive(Debug, Clone, Default)]
pub struct ResolvedContextSnapshot {
    pub task_payload: String,
    pub inherited_compact_summary: Option<String>,
    pub inherited_recent_tail: Vec<String>,
}

const DEFAULT_RECENT_TAIL_LIMIT: usize = 4;
const MAX_RECENT_TAIL_ITEMS: usize = 6;
const MAX_RECENT_TAIL_CHARS: usize = 640;

fn build_task_payload(params: &AgentExecutionRequest) -> String {
    let task = params.prompt.trim();
    let mut sections = vec![format!(
        "# Task\n{}",
        if task.is_empty() {
            "(无任务描述)"
        } else {
            task
        }
    )];
    if let Some(ctx) = params
        .context
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("# Context\n{}", ctx.trim()));
    }
    sections.join("\n\n")
}

/// 从父会话状态中提取最近的 compact summary。
pub fn latest_compact_summary(parent_state: &AgentState) -> Option<String> {
    parent_state
        .messages
        .iter()
        .rev()
        .find_map(|message| match message {
            LlmMessage::User {
                content,
                origin: UserMessageOrigin::CompactSummary,
            } => Some(
                parse_compact_summary_message(content)
                    .map(|parsed| parsed.summary)
                    .unwrap_or_else(|| content.clone()),
            ),
            _ => None,
        })
}

/// 根据覆盖配置从父会话中继承最近对话 tail。
fn inherited_recent_tail_lines(
    parent_state: &AgentState,
    overrides: &astrcode_core::ResolvedSubagentContextOverrides,
) -> Vec<String> {
    let entries = match overrides.fork_mode.as_ref() {
        Some(ForkMode::FullHistory) => parent_state
            .messages
            .iter()
            .filter_map(message_tail_entry)
            .collect(),
        Some(ForkMode::LastNTurns(turns)) => recent_tail_lines_for_turns(parent_state, *turns),
        None => parent_state
            .messages
            .iter()
            .rev()
            .filter_map(message_tail_entry)
            .take(DEFAULT_RECENT_TAIL_LIMIT)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    };

    finalize_recent_tail_entries(entries)
}

/// 获取父会话最近 N 条非空对话行（简化版）。
pub fn recent_tail_lines(parent_state: &AgentState, limit: usize) -> Vec<String> {
    let entries = parent_state
        .messages
        .iter()
        .rev()
        .filter_map(message_tail_entry)
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    finalize_recent_tail_entries(entries)
}

fn recent_tail_lines_for_turns(parent_state: &AgentState, turns: usize) -> Vec<TailEntry> {
    if turns == 0 {
        return Vec::new();
    }

    let Some(start_index) = last_n_turn_start_index(&parent_state.messages, turns) else {
        return parent_state
            .messages
            .iter()
            .filter_map(message_tail_entry)
            .collect();
    };

    parent_state.messages[start_index..]
        .iter()
        .filter_map(message_tail_entry)
        .collect()
}

fn last_n_turn_start_index(messages: &[LlmMessage], turns: usize) -> Option<usize> {
    let mut seen_turns = 0usize;

    for (index, message) in messages.iter().enumerate().rev() {
        if matches!(
            message,
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            }
        ) {
            seen_turns += 1;
            if seen_turns >= turns {
                return Some(index);
            }
        }
    }

    None
}

// ── Tail 格式化辅助 ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TailRole {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TailEntry {
    role: TailRole,
    line: String,
}

fn message_tail_entry(message: &LlmMessage) -> Option<TailEntry> {
    match message {
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::User,
        } => Some(TailEntry {
            role: TailRole::User,
            line: format!("- user: {}", single_line(content)),
        }),
        LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => Some(TailEntry {
            role: TailRole::Assistant,
            line: format!("- assistant: {}", single_line(content)),
        }),
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => Some(TailEntry {
            role: TailRole::Tool,
            line: summarize_tool_tail(tool_call_id, content),
        }),
        _ => None,
    }
}

fn finalize_recent_tail_entries(entries: Vec<TailEntry>) -> Vec<String> {
    let mut deduped = Vec::with_capacity(entries.len());
    for entry in entries {
        if deduped
            .last()
            .is_some_and(|previous: &TailEntry| previous.line == entry.line)
        {
            continue;
        }
        deduped.push(entry);
    }

    trim_recent_tail_budget(&mut deduped);
    deduped.into_iter().map(|entry| entry.line).collect()
}

fn trim_recent_tail_budget(entries: &mut Vec<TailEntry>) {
    while entries.len() > MAX_RECENT_TAIL_ITEMS || total_tail_chars(entries) > MAX_RECENT_TAIL_CHARS
    {
        if remove_oldest_entry_by_role(entries, TailRole::Tool)
            || remove_oldest_entry_by_role(entries, TailRole::Assistant)
            || remove_oldest_entry_by_role(entries, TailRole::User)
        {
            continue;
        }
        break;
    }
}

fn remove_oldest_entry_by_role(entries: &mut Vec<TailEntry>, role: TailRole) -> bool {
    let Some(index) = entries.iter().position(|entry| entry.role == role) else {
        return false;
    };
    entries.remove(index);
    true
}

fn total_tail_chars(entries: &[TailEntry]) -> usize {
    entries.iter().map(|entry| entry.line.len()).sum()
}

fn summarize_tool_tail(tool_call_id: &str, content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return format!("- tool[{tool_call_id}]: (empty output)");
    }

    let line_count = trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
        .max(1);
    let excerpt = truncate_with_ellipsis(&single_line(trimmed), 96);
    if trimmed.len() > 120 || line_count > 3 {
        format!(
            "- tool[{tool_call_id}]: summary: {excerpt} ({} chars / {} lines)",
            trimmed.len(),
            line_count
        )
    } else {
        format!("- tool[{tool_call_id}]: {excerpt}")
    }
}

fn truncate_with_ellipsis(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }

    let mut end = content.len();
    for (index, _) in content.char_indices().take(max_chars) {
        end = index;
    }
    format!("{}...", &content[..end])
}

/// 将多行内容压缩为单行摘要。
pub fn single_line(content: &str) -> String {
    let collapsed = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.len() > 200 {
        let mut end = 200;
        while !collapsed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &collapsed[..end])
    } else {
        collapsed
    }
}

// ── 执行谱系索引 ─────────────────────────────────────────────

pub const LINEAGE_METADATA_UNAVAILABLE_MESSAGE: &str =
    "lineage metadata unavailable for requested scope";

/// 上下文继承 block ID 常量，供 prompt 声明使用。
pub const CHILD_INHERITED_COMPACT_SUMMARY_BLOCK_ID: &str = "child.inherited.compact_summary";
pub const CHILD_INHERITED_RECENT_TAIL_BLOCK_ID: &str = "child.inherited.recent_tail";

/// 谱系查询范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionLineageScope {
    SelfOnly,
    DirectChildren,
    Subtree,
}

/// 单个执行节点的谱系信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLineageEntry {
    pub sub_run_id: String,
    pub agent_id: Option<String>,
    pub parent_sub_run_id: Option<String>,
}

/// 执行谱系索引：维护 sub_run 之间的父子关系。
///
/// 从事件流中增量构建，支持跨会话的血缘追踪。
/// 为什么在 kernel 而不是 session-runtime：谱系关系是跨会话的，
/// 多个子会话共享同一个谱系索引。
#[derive(Debug, Clone, Default)]
pub struct ExecutionLineageIndex {
    by_sub_run_id: HashMap<String, ExecutionLineageEntry>,
    /// turn_id → 拥有该 turn 的 sub_run_id。
    turn_to_sub_run: HashMap<String, String>,
    children_by_parent_sub_run: HashMap<String, Vec<String>>,
}

impl ExecutionLineageIndex {
    pub fn from_session_history(history: &[SessionEventRecord]) -> Self {
        let mut index = Self::default();
        for record in history {
            index.observe_agent_event(&record.event);
        }
        index
    }

    pub fn from_stored_events(events: &[StoredEvent]) -> Self {
        let mut index = Self::default();
        for stored in events {
            index.observe_storage_event(&stored.event);
        }
        index
    }

    pub fn observe_session_record(&mut self, record: &SessionEventRecord) {
        self.observe_agent_event(&record.event);
    }

    pub fn observe_stored_event(&mut self, stored: &StoredEvent) {
        self.observe_storage_event(&stored.event);
    }

    pub fn contains(&self, sub_run_id: &str) -> bool {
        self.by_sub_run_id.contains_key(sub_run_id)
    }

    pub fn require_scope(
        &self,
        sub_run_id: &str,
        scope: ExecutionLineageScope,
    ) -> Result<(), String> {
        if matches!(scope, ExecutionLineageScope::SelfOnly) {
            return Ok(());
        }
        if !self.by_sub_run_id.contains_key(sub_run_id) {
            return Err(LINEAGE_METADATA_UNAVAILABLE_MESSAGE.to_string());
        }
        let has_turn_ownership = self
            .turn_to_sub_run
            .values()
            .any(|owned_sub_run_id| owned_sub_run_id == sub_run_id);
        if !has_turn_ownership {
            return Err(LINEAGE_METADATA_UNAVAILABLE_MESSAGE.to_string());
        }
        Ok(())
    }

    pub fn is_direct_child_of(&self, sub_run_id: &str, target_sub_run_id: &str) -> bool {
        self.by_sub_run_id
            .get(sub_run_id)
            .and_then(|entry| entry.parent_sub_run_id.as_deref())
            == Some(target_sub_run_id)
    }

    pub fn is_in_subtree(&self, sub_run_id: &str, target_sub_run_id: &str) -> bool {
        if sub_run_id == target_sub_run_id {
            return true;
        }

        let mut current = self
            .by_sub_run_id
            .get(sub_run_id)
            .and_then(|entry| entry.parent_sub_run_id.as_deref());
        while let Some(parent_sub_run_id) = current {
            if parent_sub_run_id == target_sub_run_id {
                return true;
            }
            current = self
                .by_sub_run_id
                .get(parent_sub_run_id)
                .and_then(|entry| entry.parent_sub_run_id.as_deref());
        }
        false
    }

    pub fn direct_children_of(&self, target_sub_run_id: &str) -> Vec<String> {
        self.children_by_parent_sub_run
            .get(target_sub_run_id)
            .cloned()
            .unwrap_or_default()
    }

    fn observe_storage_event(&mut self, event: &StorageEvent) {
        match &event.payload {
            StorageEventPayload::SubRunStarted { .. }
            | StorageEventPayload::SubRunFinished { .. } => {
                let agent = &event.agent;
                self.observe_lifecycle(
                    agent.sub_run_id.as_deref(),
                    agent.agent_id.as_deref(),
                    agent.parent_sub_run_id.as_deref(),
                );
            },
            _ => {
                if let (Some(turn_id), Some(agent)) = (event.turn_id(), event.agent_context()) {
                    self.observe_turn_owner(turn_id, agent.sub_run_id.as_deref());
                }
            },
        }
    }

    /// 增量吸收一条 live / durable AgentEvent。
    pub fn observe_agent_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::SubRunStarted { agent, .. } | AgentEvent::SubRunFinished { agent, .. } => {
                self.observe_lifecycle(
                    agent.sub_run_id.as_deref(),
                    agent.agent_id.as_deref(),
                    agent.parent_sub_run_id.as_deref(),
                );
            },
            AgentEvent::UserMessage { turn_id, agent, .. }
            | AgentEvent::ModelDelta { turn_id, agent, .. }
            | AgentEvent::ThinkingDelta { turn_id, agent, .. }
            | AgentEvent::AssistantMessage { turn_id, agent, .. }
            | AgentEvent::ToolCallStart { turn_id, agent, .. }
            | AgentEvent::ToolCallDelta { turn_id, agent, .. }
            | AgentEvent::ToolCallResult { turn_id, agent, .. }
            | AgentEvent::TurnDone { turn_id, agent } => {
                self.observe_turn_owner(turn_id, agent.sub_run_id.as_deref());
            },
            AgentEvent::PhaseChanged { turn_id, agent, .. }
            | AgentEvent::PromptMetrics { turn_id, agent, .. }
            | AgentEvent::CompactApplied { turn_id, agent, .. }
            | AgentEvent::ChildSessionNotification { turn_id, agent, .. }
            | AgentEvent::Error { turn_id, agent, .. } => {
                if let Some(turn_id) = turn_id.as_deref() {
                    self.observe_turn_owner(turn_id, agent.sub_run_id.as_deref());
                }
            },
            AgentEvent::AgentMailboxQueued { .. }
            | AgentEvent::AgentMailboxBatchStarted { .. }
            | AgentEvent::AgentMailboxBatchAcked { .. }
            | AgentEvent::AgentMailboxDiscarded { .. } => {},
            AgentEvent::SessionStarted { .. } => {},
        }
    }

    fn observe_turn_owner(&mut self, turn_id: &str, sub_run_id: Option<&str>) {
        let Some(sub_run_id) = sub_run_id else {
            return;
        };
        self.turn_to_sub_run
            .insert(turn_id.to_string(), sub_run_id.to_string());
    }

    /// 注册一个 sub_run 的生命周期事件。
    fn observe_lifecycle(
        &mut self,
        sub_run_id: Option<&str>,
        agent_id: Option<&str>,
        parent_sub_run_id: Option<&str>,
    ) {
        let Some(sub_run_id) = sub_run_id else {
            return;
        };
        let entry = self
            .by_sub_run_id
            .entry(sub_run_id.to_string())
            .or_insert_with(|| ExecutionLineageEntry {
                sub_run_id: sub_run_id.to_string(),
                agent_id: None,
                parent_sub_run_id: None,
            });
        let previous_parent = entry.parent_sub_run_id.clone();

        if let Some(agent_id) = agent_id {
            entry.agent_id = Some(agent_id.to_string());
        }

        let next_parent = parent_sub_run_id
            .filter(|parent_sub_run| *parent_sub_run != sub_run_id)
            .map(ToString::to_string);
        entry.parent_sub_run_id = next_parent.clone();

        if previous_parent != next_parent {
            if let Some(previous_parent) = previous_parent.as_deref() {
                self.remove_child_link(previous_parent, sub_run_id);
            }
            if let Some(new_parent) = next_parent.as_deref() {
                self.add_child_link(new_parent, sub_run_id);
            }
        }
    }

    fn add_child_link(&mut self, parent_sub_run_id: &str, child_sub_run_id: &str) {
        let children = self
            .children_by_parent_sub_run
            .entry(parent_sub_run_id.to_string())
            .or_default();
        match children.binary_search_by(|existing| existing.as_str().cmp(child_sub_run_id)) {
            Ok(_) => {},
            Err(index) => children.insert(index, child_sub_run_id.to_string()),
        }
    }

    fn remove_child_link(&mut self, parent_sub_run_id: &str, child_sub_run_id: &str) {
        let should_remove_parent =
            if let Some(children) = self.children_by_parent_sub_run.get_mut(parent_sub_run_id) {
                if let Ok(index) =
                    children.binary_search_by(|existing| existing.as_str().cmp(child_sub_run_id))
                {
                    children.remove(index);
                }
                children.is_empty()
            } else {
                false
            };

        if should_remove_parent {
            self.children_by_parent_sub_run.remove(parent_sub_run_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEvent, AgentEventContext, AgentState, ForkMode, LlmMessage,
        ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, SessionEventRecord,
        UserMessageOrigin, format_compact_summary,
    };

    use super::{
        ExecutionLineageIndex, ExecutionLineageScope, LINEAGE_METADATA_UNAVAILABLE_MESSAGE,
        latest_compact_summary, recent_tail_lines, resolve_context_snapshot, single_line,
    };
    use crate::execution::prep::AgentExecutionRequest;

    #[test]
    fn resolve_context_snapshot_inherits_summary_and_tail_when_enabled() {
        let parent_state = AgentState {
            messages: vec![
                LlmMessage::User {
                    content: "summary one".to_string(),
                    origin: UserMessageOrigin::CompactSummary,
                },
                LlmMessage::User {
                    content: "user question".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "assistant answer".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
            ],
            ..AgentState::default()
        };
        let request = AgentExecutionRequest {
            subagent_type: Some("explore".to_string()),
            description: "investigate issue".to_string(),
            prompt: "investigate issue".to_string(),
            context: Some("focus on regressions".to_string()),
            context_overrides: None,
        };
        let overrides = ResolvedSubagentContextOverrides {
            include_compact_summary: true,
            include_recent_tail: true,
            ..ResolvedSubagentContextOverrides::default()
        };

        let snapshot = resolve_context_snapshot(&request, Some(&parent_state), &overrides);

        assert!(snapshot.task_payload.contains("# Task\ninvestigate issue"));
        assert!(
            snapshot
                .task_payload
                .contains("# Context\nfocus on regressions")
        );
        assert_eq!(
            snapshot.inherited_compact_summary.as_deref(),
            Some("summary one")
        );
        assert_eq!(snapshot.inherited_recent_tail.len(), 2);
    }

    #[test]
    fn resolve_context_snapshot_omits_parent_data_when_disabled() {
        let parent_state = AgentState {
            messages: vec![LlmMessage::User {
                content: "summary one".to_string(),
                origin: UserMessageOrigin::CompactSummary,
            }],
            ..AgentState::default()
        };
        let request = AgentExecutionRequest {
            subagent_type: None,
            description: "investigate issue".to_string(),
            prompt: "investigate issue".to_string(),
            context: None,
            context_overrides: None,
        };
        let overrides = ResolvedSubagentContextOverrides {
            include_compact_summary: false,
            include_recent_tail: false,
            ..ResolvedSubagentContextOverrides::default()
        };

        let snapshot = resolve_context_snapshot(&request, Some(&parent_state), &overrides);

        assert!(snapshot.inherited_compact_summary.is_none());
        assert!(snapshot.inherited_recent_tail.is_empty());
    }

    #[test]
    fn latest_compact_summary_picks_latest_compact_message() {
        let parent_state = AgentState {
            messages: vec![
                LlmMessage::User {
                    content: format_compact_summary("old summary"),
                    origin: UserMessageOrigin::CompactSummary,
                },
                LlmMessage::User {
                    content: format_compact_summary("new summary"),
                    origin: UserMessageOrigin::CompactSummary,
                },
            ],
            ..AgentState::default()
        };

        assert_eq!(
            latest_compact_summary(&parent_state).as_deref(),
            Some("new summary")
        );
    }

    #[test]
    fn recent_tail_lines_preserves_order_and_filters_empty_assistant() {
        let parent_state = AgentState {
            messages: vec![
                LlmMessage::User {
                    content: "a".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: " ".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
                LlmMessage::Assistant {
                    content: "b".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
            ],
            ..AgentState::default()
        };

        let lines = recent_tail_lines(&parent_state, 4);
        assert_eq!(lines, vec!["- user: a", "- assistant: b"]);
    }

    #[test]
    fn single_line_truncates_long_multiline_content() {
        let content = format!("line1\n{}", "x".repeat(260));
        let one_line = single_line(&content);

        assert!(one_line.len() <= 203);
        assert!(one_line.ends_with("..."));
        assert!(!one_line.contains('\n'));
    }

    #[test]
    fn resolve_context_snapshot_honors_fork_mode_last_n_turns() {
        let parent_state = AgentState {
            messages: vec![
                LlmMessage::User {
                    content: "turn-1 user".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "turn-1 answer".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
                LlmMessage::User {
                    content: "turn-2 user".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "turn-2 answer".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
                LlmMessage::User {
                    content: "turn-3 user".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "turn-3 answer".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
            ],
            ..AgentState::default()
        };
        let request = AgentExecutionRequest {
            subagent_type: Some("explore".to_string()),
            description: "investigate issue".to_string(),
            prompt: "investigate issue".to_string(),
            context: None,
            context_overrides: None,
        };
        let overrides = ResolvedSubagentContextOverrides {
            include_recent_tail: true,
            fork_mode: Some(ForkMode::LastNTurns(2)),
            ..ResolvedSubagentContextOverrides::default()
        };

        let snapshot = resolve_context_snapshot(&request, Some(&parent_state), &overrides);

        assert_eq!(
            snapshot.inherited_recent_tail,
            vec![
                "- user: turn-2 user",
                "- assistant: turn-2 answer",
                "- user: turn-3 user",
                "- assistant: turn-3 answer",
            ]
        );
    }

    fn lineage_record(event_id: &str, event: AgentEvent) -> SessionEventRecord {
        SessionEventRecord {
            event_id: event_id.to_string(),
            event,
        }
    }

    #[test]
    fn execution_lineage_index_tracks_direct_children_from_events() {
        let history = vec![
            lineage_record(
                "1.0",
                AgentEvent::SubRunStarted {
                    turn_id: Some("turn-root".to_string()),
                    agent: AgentEventContext::sub_run(
                        "agent-a",
                        "turn-root",
                        "review",
                        "sub-a",
                        None,
                        astrcode_core::SubRunStorageMode::IndependentSession,
                        None,
                    ),
                    tool_call_id: None,
                    resolved_overrides: ResolvedSubagentContextOverrides::default(),
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
            lineage_record(
                "2.0",
                AgentEvent::SubRunStarted {
                    turn_id: Some("turn-a".to_string()),
                    agent: AgentEventContext::sub_run(
                        "agent-b",
                        "turn-a",
                        "review",
                        "sub-b",
                        Some("sub-a".to_string()),
                        astrcode_core::SubRunStorageMode::IndependentSession,
                        None,
                    ),
                    tool_call_id: None,
                    resolved_overrides: ResolvedSubagentContextOverrides::default(),
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
        ];

        let index = ExecutionLineageIndex::from_session_history(&history);

        assert!(index.contains("sub-a"));
        assert!(index.contains("sub-b"));
    }

    #[test]
    fn execution_lineage_index_rejects_non_self_scope_when_lineage_metadata_is_missing() {
        let history = vec![lineage_record(
            "1.0",
            AgentEvent::SubRunStarted {
                turn_id: Some("turn-legacy".to_string()),
                agent: AgentEventContext::sub_run(
                    "agent-legacy",
                    "turn-legacy",
                    "review",
                    "sub-legacy",
                    None,
                    astrcode_core::SubRunStorageMode::IndependentSession,
                    None,
                ),
                tool_call_id: None,
                resolved_overrides: ResolvedSubagentContextOverrides::default(),
                resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
            },
        )];

        let index = ExecutionLineageIndex::from_session_history(&history);

        assert_eq!(
            index.require_scope("sub-legacy", ExecutionLineageScope::DirectChildren),
            Err(LINEAGE_METADATA_UNAVAILABLE_MESSAGE.to_string())
        );
        assert_eq!(
            index.require_scope("sub-legacy", ExecutionLineageScope::SelfOnly),
            Ok(())
        );
    }
}
