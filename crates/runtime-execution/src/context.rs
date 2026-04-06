use astrcode_core::{AgentState, LlmMessage, UserMessageOrigin};
use astrcode_runtime_agent_tool::RunAgentParams;

pub fn resolve_context_snapshot(
    params: &RunAgentParams,
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

    let mut sections = vec![format!("# Task\n{}", params.task.trim())];
    if let Some(context) = params
        .context
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("# Context\n{}", context.trim()));
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
        composed_task: sections.join("\n\n"),
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
