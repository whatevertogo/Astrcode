//! 子 Agent 上下文解析模块。
//!
//! 负责将父会话状态和子 Agent 参数组合成完整的上下文快照，包括：
//! - 任务主体（prompt）和补充上下文（context）
//! - 父会话的 compact summary 继承
//! - 父会话的最近 N 轮对话 tail 继承
//!
//! 设计原则：纯函数无状态，便于测试和复用。

use astrcode_core::{AgentState, LlmMessage, UserMessageOrigin};

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
            .map(|state| recent_tail_lines(state, 4))
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
            } => Some(content.clone()),
            _ => None,
        })
}

pub fn recent_tail_lines(parent_state: &AgentState, limit: usize) -> Vec<String> {
    parent_state
        .messages
        .iter()
        .rev()
        .filter_map(|message| match message {
            LlmMessage::User { content, .. } => Some(format!("- user: {}", single_line(content))),
            LlmMessage::Assistant { content, .. } if !content.trim().is_empty() => {
                Some(format!("- assistant: {}", single_line(content)))
            },
            LlmMessage::Tool {
                tool_call_id,
                content,
            } => Some(format!("- tool[{tool_call_id}]: {}", single_line(content))),
            _ => None,
        })
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
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

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentState, LlmMessage, ResolvedSubagentContextOverrides, UserMessageOrigin,
    };

    use super::{latest_compact_summary, recent_tail_lines, resolve_context_snapshot, single_line};
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
            max_steps: None,
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
        assert_eq!(snapshot.inherited_recent_tail.len(), 3);
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
            max_steps: None,
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
                    content: "old summary".to_string(),
                    origin: UserMessageOrigin::CompactSummary,
                },
                LlmMessage::User {
                    content: "new summary".to_string(),
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
}
