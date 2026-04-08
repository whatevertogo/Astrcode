//! 子 Agent 上下文解析模块。
//!
//! 负责将父会话状态和子 Agent 参数组合成完整的上下文快照，包括：
//! - 任务主体（prompt）和补充上下文（context）
//! - 父会话的 compact summary 继承
//! - 父会话的最近 N 轮对话 tail 继承
//!
//! 设计原则：纯函数无状态，便于测试和复用。

use std::collections::{HashMap, HashSet};

use astrcode_core::{
    AgentEvent, AgentState, ForkMode, LlmMessage, SessionEventRecord, StorageEvent, StoredEvent,
    SubRunDescriptor, UserMessageOrigin, parse_compact_summary_message,
};

use crate::AgentExecutionRequest;

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

    let mut sections = Vec::new();
    // 从扁平字段读取：prompt 是任务主体，context 是可选补充
    sections.push(format!("# Task\n{}", params.prompt.trim()));
    if let Some(ctx) = params.context.as_deref().filter(|s| !s.trim().is_empty()) {
        sections.push(format!("# Context\n{}", ctx.trim()));
    }
    if let Some(summary) = inherited_compact_summary.as_ref() {
        sections.push(format!("# Parent Compact Summary\n{}", summary.trim()));
    }
    if !inherited_recent_tail.is_empty() {
        sections.push(format!(
            "# Recent Tail\n{}",
            inherited_recent_tail.join("\n")
        ));
    }

    ResolvedContextSnapshot {
        composed_task: if sections.is_empty() {
            "# Task\n(无任务描述)".to_string()
        } else {
            sections.join("\n\n")
        },
        inherited_compact_summary,
        inherited_recent_tail,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedContextSnapshot {
    pub composed_task: String,
    pub inherited_compact_summary: Option<String>,
    pub inherited_recent_tail: Vec<String>,
}

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

fn inherited_recent_tail_lines(
    parent_state: &AgentState,
    overrides: &astrcode_core::ResolvedSubagentContextOverrides,
) -> Vec<String> {
    match overrides.fork_mode.as_ref() {
        Some(ForkMode::FullHistory) => parent_state
            .messages
            .iter()
            .filter_map(message_tail_line)
            .collect(),
        Some(ForkMode::LastNTurns(turns)) => recent_tail_lines_for_turns(parent_state, *turns),
        None => recent_tail_lines(parent_state, 4),
    }
}

pub fn recent_tail_lines(parent_state: &AgentState, limit: usize) -> Vec<String> {
    parent_state
        .messages
        .iter()
        .rev()
        .filter_map(message_tail_line)
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn recent_tail_lines_for_turns(parent_state: &AgentState, turns: usize) -> Vec<String> {
    if turns == 0 {
        return Vec::new();
    }

    let Some(start_index) = last_n_turn_start_index(&parent_state.messages, turns) else {
        return parent_state
            .messages
            .iter()
            .filter_map(message_tail_line)
            .collect();
    };

    parent_state.messages[start_index..]
        .iter()
        .filter_map(message_tail_line)
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

fn message_tail_line(message: &LlmMessage) -> Option<String> {
    match message {
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::User,
        } => Some(format!("- user: {}", single_line(content))),
        LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
            Some(format!("- assistant: {}", single_line(content)))
        },
        LlmMessage::Tool {
            tool_call_id,
            content,
        } => Some(format!("- tool[{tool_call_id}]: {}", single_line(content))),
        _ => None,
    }
}

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

pub const LINEAGE_METADATA_UNAVAILABLE_MESSAGE: &str =
    "lineage metadata unavailable for requested scope";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionLineageScope {
    SelfOnly,
    DirectChildren,
    Subtree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLineageEntry {
    pub sub_run_id: String,
    pub agent_id: Option<String>,
    pub descriptor: Option<SubRunDescriptor>,
    pub parent_sub_run_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionLineageIndex {
    by_sub_run_id: HashMap<String, ExecutionLineageEntry>,
    agent_to_sub_run: HashMap<String, String>,
    children_by_parent_sub_run: HashMap<String, Vec<String>>,
    legacy_gap_sub_run_ids: HashSet<String>,
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

    pub fn has_legacy_gap(&self) -> bool {
        !self.legacy_gap_sub_run_ids.is_empty()
    }

    pub fn require_scope(
        &self,
        sub_run_id: &str,
        scope: ExecutionLineageScope,
    ) -> Result<(), String> {
        if matches!(scope, ExecutionLineageScope::SelfOnly) {
            return Ok(());
        }
        if self.legacy_gap_sub_run_ids.contains(sub_run_id) || self.has_legacy_gap() {
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
        match event {
            StorageEvent::SubRunStarted {
                agent, descriptor, ..
            }
            | StorageEvent::SubRunFinished {
                agent, descriptor, ..
            } => self.observe_lifecycle(
                agent.sub_run_id.as_deref(),
                agent.agent_id.as_deref(),
                descriptor.as_ref(),
            ),
            _ => {},
        }
    }

    fn observe_agent_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::SubRunStarted {
                agent, descriptor, ..
            }
            | AgentEvent::SubRunFinished {
                agent, descriptor, ..
            } => self.observe_lifecycle(
                agent.sub_run_id.as_deref(),
                agent.agent_id.as_deref(),
                descriptor.as_ref(),
            ),
            _ => {},
        }
    }

    fn observe_lifecycle(
        &mut self,
        sub_run_id: Option<&str>,
        agent_id: Option<&str>,
        descriptor: Option<&SubRunDescriptor>,
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
                descriptor: None,
                parent_sub_run_id: None,
            });

        if let Some(agent_id) = agent_id {
            entry.agent_id = Some(agent_id.to_string());
            self.agent_to_sub_run
                .insert(agent_id.to_string(), sub_run_id.to_string());
        }

        if let Some(descriptor) = descriptor {
            entry.descriptor = Some(descriptor.clone());
            entry.parent_sub_run_id = descriptor
                .parent_agent_id
                .as_deref()
                .and_then(|parent_agent_id| self.agent_to_sub_run.get(parent_agent_id))
                .cloned();
            self.legacy_gap_sub_run_ids.remove(sub_run_id);
        } else if entry.descriptor.is_none() {
            self.legacy_gap_sub_run_ids.insert(sub_run_id.to_string());
        }

        self.rebuild_children_index();
    }

    fn rebuild_children_index(&mut self) {
        let mut children_by_parent_sub_run: HashMap<String, Vec<String>> = HashMap::new();
        for entry in self.by_sub_run_id.values() {
            let Some(parent_sub_run_id) = entry.parent_sub_run_id.as_ref() else {
                continue;
            };
            children_by_parent_sub_run
                .entry(parent_sub_run_id.clone())
                .or_default()
                .push(entry.sub_run_id.clone());
        }
        for children in children_by_parent_sub_run.values_mut() {
            children.sort();
            children.dedup();
        }
        self.children_by_parent_sub_run = children_by_parent_sub_run;
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentEvent, AgentEventContext, AgentState, ForkMode, LlmMessage,
        ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, SessionEventRecord,
        SubRunDescriptor, UserMessageOrigin, format_compact_summary,
    };

    use super::{
        ExecutionLineageIndex, ExecutionLineageScope, LINEAGE_METADATA_UNAVAILABLE_MESSAGE,
        latest_compact_summary, recent_tail_lines, resolve_context_snapshot, single_line,
    };
    use crate::AgentExecutionRequest;

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

        assert!(snapshot.composed_task.contains("# Task\ninvestigate issue"));
        assert!(
            snapshot
                .composed_task
                .contains("# Context\nfocus on regressions")
        );
        assert!(
            snapshot
                .composed_task
                .contains("# Parent Compact Summary\nsummary one")
        );
        assert!(snapshot.composed_task.contains("# Recent Tail\n"));
        assert_eq!(
            snapshot.inherited_compact_summary.as_deref(),
            Some("summary one")
        );
        assert_eq!(snapshot.inherited_recent_tail.len(), 2);
        assert_eq!(
            snapshot.inherited_recent_tail,
            vec!["- user: user question", "- assistant: assistant answer"]
        );
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

        assert!(!snapshot.composed_task.contains("Parent Compact Summary"));
        assert!(!snapshot.composed_task.contains("Recent Tail"));
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
    fn recent_tail_lines_skips_internal_user_protocol_messages() {
        let parent_state = AgentState {
            messages: vec![
                LlmMessage::User {
                    content: format_compact_summary("summary"),
                    origin: UserMessageOrigin::CompactSummary,
                },
                LlmMessage::User {
                    content: "Continue from where you left off.".to_string(),
                    origin: UserMessageOrigin::AutoContinueNudge,
                },
                LlmMessage::User {
                    content: "actual user".to_string(),
                    origin: UserMessageOrigin::User,
                },
                LlmMessage::Assistant {
                    content: "answer".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                },
            ],
            ..AgentState::default()
        };

        let lines = recent_tail_lines(&parent_state, 8);

        assert_eq!(lines, vec!["- user: actual user", "- assistant: answer"]);
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

    #[test]
    fn single_line_truncates_long_multiline_content() {
        let content = format!("line1\n{}", "x".repeat(260));
        let one_line = single_line(&content);

        assert!(one_line.len() <= 203);
        assert!(one_line.ends_with("..."));
        assert!(!one_line.contains('\n'));
    }

    fn lineage_record(event_id: &str, event: AgentEvent) -> SessionEventRecord {
        SessionEventRecord {
            event_id: event_id.to_string(),
            event,
        }
    }

    #[test]
    fn execution_lineage_index_tracks_direct_children_from_descriptors() {
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
                        astrcode_core::SubRunStorageMode::SharedSession,
                        None,
                    ),
                    descriptor: Some(SubRunDescriptor {
                        sub_run_id: "sub-a".to_string(),
                        parent_turn_id: "turn-root".to_string(),
                        parent_agent_id: None,
                        depth: 1,
                    }),
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
                        astrcode_core::SubRunStorageMode::SharedSession,
                        None,
                    ),
                    descriptor: Some(SubRunDescriptor {
                        sub_run_id: "sub-b".to_string(),
                        parent_turn_id: "turn-a".to_string(),
                        parent_agent_id: Some("agent-a".to_string()),
                        depth: 2,
                    }),
                    tool_call_id: None,
                    resolved_overrides: ResolvedSubagentContextOverrides::default(),
                    resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                },
            ),
        ];

        let index = ExecutionLineageIndex::from_session_history(&history);

        assert!(index.contains("sub-a"));
        assert!(index.contains("sub-b"));
        assert!(index.is_direct_child_of("sub-b", "sub-a"));
        assert!(index.is_in_subtree("sub-b", "sub-a"));
        assert_eq!(index.direct_children_of("sub-a"), vec!["sub-b".to_string()]);
    }

    #[test]
    fn execution_lineage_index_rejects_non_self_scope_when_legacy_gap_exists() {
        let history = vec![lineage_record(
            "1.0",
            AgentEvent::SubRunStarted {
                turn_id: Some("turn-legacy".to_string()),
                agent: AgentEventContext::sub_run(
                    "agent-legacy",
                    "turn-legacy",
                    "review",
                    "sub-legacy",
                    astrcode_core::SubRunStorageMode::SharedSession,
                    None,
                ),
                descriptor: None,
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
            index.require_scope("sub-legacy", ExecutionLineageScope::Subtree),
            Err(LINEAGE_METADATA_UNAVAILABLE_MESSAGE.to_string())
        );
        assert_eq!(
            index.require_scope("sub-legacy", ExecutionLineageScope::SelfOnly),
            Ok(())
        );
    }
}
