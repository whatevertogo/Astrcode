//! 父级上下文继承：子代理启动时从父 session 继承消息。
//!
//! 支持两种继承策略：
//! - **Compact summary**：从父消息中提取压缩摘要，给子代理一个精简的上下文概览
//! - **Recent tail**：按 fork mode 截取父消息尾部（LastNTurns 或 FullHistory）

use astrcode_core::{
    ForkMode, LlmMessage, ResolvedSubagentContextOverrides, UserMessageOrigin, project,
};

use crate::{AgentSessionPort, ApplicationError};

pub(crate) async fn resolve_inherited_parent_messages(
    session_runtime: &dyn AgentSessionPort,
    parent_session_id: &str,
    overrides: &ResolvedSubagentContextOverrides,
) -> Result<Vec<LlmMessage>, ApplicationError> {
    let parent_events = session_runtime
        .session_stored_events(&astrcode_core::SessionId::from(
            parent_session_id.to_string(),
        ))
        .await
        .map_err(ApplicationError::from)?;
    let projected = project(
        &parent_events
            .iter()
            .map(|stored| stored.event.clone())
            .collect::<Vec<_>>(),
    );
    Ok(build_inherited_messages(&projected.messages, overrides))
}

pub(crate) fn build_inherited_messages(
    parent_messages: &[LlmMessage],
    overrides: &ResolvedSubagentContextOverrides,
) -> Vec<LlmMessage> {
    let mut inherited = Vec::new();

    if overrides.include_compact_summary {
        if let Some(summary) = parent_messages.iter().find(|message| {
            matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::CompactSummary,
                    ..
                }
            )
        }) {
            inherited.push(summary.clone());
        }
    }

    if overrides.include_recent_tail {
        inherited.extend(select_inherited_recent_tail(
            parent_messages,
            overrides.fork_mode.as_ref(),
        ));
    }

    inherited
}

/// 从父消息中选择要继承的最近尾部。
/// 先排除 CompactSummary 消息（已单独处理），再按 fork_mode 截取。
pub(crate) fn select_inherited_recent_tail(
    parent_messages: &[LlmMessage],
    fork_mode: Option<&ForkMode>,
) -> Vec<LlmMessage> {
    let non_summary_messages = parent_messages
        .iter()
        .filter(|message| {
            !matches!(
                message,
                LlmMessage::User {
                    origin: UserMessageOrigin::CompactSummary,
                    ..
                }
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    match fork_mode {
        Some(ForkMode::LastNTurns(turns)) => {
            tail_messages_for_last_n_turns(&non_summary_messages, *turns)
        },
        Some(ForkMode::FullHistory) | None => non_summary_messages,
    }
}

/// 从尾部倒数 `turns` 个 User 消息作为 turn 边界，截取最近的 N 个 turn。
fn tail_messages_for_last_n_turns(messages: &[LlmMessage], turns: usize) -> Vec<LlmMessage> {
    if turns == 0 || messages.is_empty() {
        return Vec::new();
    }

    let mut remaining_turns = turns;
    let mut start_index = 0usize;
    for (index, message) in messages.iter().enumerate().rev() {
        if matches!(
            message,
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            }
        ) {
            remaining_turns = remaining_turns.saturating_sub(1);
            start_index = index;
            if remaining_turns == 0 {
                break;
            }
        }
    }

    messages[start_index..].to_vec()
}
