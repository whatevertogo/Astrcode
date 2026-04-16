use std::collections::BTreeSet;

use astrcode_client::{
    AstrcodeConversationAgentLifecycleDto, AstrcodeConversationBlockDto,
    AstrcodeConversationBlockStatusDto,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptCell {
    pub id: String,
    pub expanded: bool,
    pub kind: TranscriptCellKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptCellStatus {
    Streaming,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptCellKind {
    User {
        body: String,
    },
    Assistant {
        body: String,
        status: TranscriptCellStatus,
    },
    Thinking {
        body: String,
        status: TranscriptCellStatus,
    },
    ToolCall {
        tool_name: String,
        summary: String,
        status: TranscriptCellStatus,
        stdout: String,
        stderr: String,
        error: Option<String>,
        duration_ms: Option<u64>,
        truncated: bool,
        child_session_id: Option<String>,
    },
    Error {
        code: String,
        message: String,
    },
    SystemNote {
        note_kind: String,
        markdown: String,
    },
    ChildHandoff {
        handoff_kind: String,
        title: String,
        lifecycle: AstrcodeConversationAgentLifecycleDto,
        message: String,
        child_session_id: String,
        child_agent_id: String,
    },
}

impl TranscriptCell {
    pub fn from_block(
        block: &AstrcodeConversationBlockDto,
        expanded_ids: &BTreeSet<String>,
    ) -> Self {
        let id = match block {
            AstrcodeConversationBlockDto::User(block) => block.id.clone(),
            AstrcodeConversationBlockDto::Assistant(block) => block.id.clone(),
            AstrcodeConversationBlockDto::Thinking(block) => block.id.clone(),
            AstrcodeConversationBlockDto::ToolCall(block) => block.id.clone(),
            AstrcodeConversationBlockDto::Error(block) => block.id.clone(),
            AstrcodeConversationBlockDto::SystemNote(block) => block.id.clone(),
            AstrcodeConversationBlockDto::ChildHandoff(block) => block.id.clone(),
        };
        let expanded = expanded_ids.contains(&id)
            || matches!(
                block,
                AstrcodeConversationBlockDto::Thinking(thinking)
                    if matches!(thinking.status, AstrcodeConversationBlockStatusDto::Streaming)
            );
        match block {
            AstrcodeConversationBlockDto::User(block) => Self {
                id,
                expanded,
                kind: TranscriptCellKind::User {
                    body: block.markdown.clone(),
                },
            },
            AstrcodeConversationBlockDto::Assistant(block) => Self {
                id,
                expanded,
                kind: TranscriptCellKind::Assistant {
                    body: block.markdown.clone(),
                    status: block.status.into(),
                },
            },
            AstrcodeConversationBlockDto::Thinking(block) => Self {
                id,
                expanded,
                kind: TranscriptCellKind::Thinking {
                    body: block.markdown.clone(),
                    status: block.status.into(),
                },
            },
            AstrcodeConversationBlockDto::ToolCall(block) => Self {
                id,
                expanded,
                kind: TranscriptCellKind::ToolCall {
                    tool_name: block.tool_name.clone(),
                    summary: block
                        .summary
                        .clone()
                        .or_else(|| block.error.clone())
                        .or_else(|| {
                            if block.streams.stdout.is_empty() && block.streams.stderr.is_empty() {
                                None
                            } else {
                                Some("工具输出已更新".to_string())
                            }
                        })
                        .clone()
                        .unwrap_or_else(|| "正在执行工具调用".to_string()),
                    status: block.status.into(),
                    stdout: block.streams.stdout.clone(),
                    stderr: block.streams.stderr.clone(),
                    error: block.error.clone(),
                    duration_ms: block.duration_ms,
                    truncated: block.truncated,
                    child_session_id: block
                        .child_ref
                        .as_ref()
                        .map(|child_ref| child_ref.open_session_id.clone()),
                },
            },
            AstrcodeConversationBlockDto::Error(block) => Self {
                id,
                expanded,
                kind: TranscriptCellKind::Error {
                    code: format!("{:?}", block.code),
                    message: block.message.clone(),
                },
            },
            AstrcodeConversationBlockDto::SystemNote(block) => Self {
                id,
                expanded,
                kind: TranscriptCellKind::SystemNote {
                    note_kind: format!("{:?}", block.note_kind),
                    markdown: block.markdown.clone(),
                },
            },
            AstrcodeConversationBlockDto::ChildHandoff(block) => Self {
                id,
                expanded,
                kind: TranscriptCellKind::ChildHandoff {
                    handoff_kind: format!("{:?}", block.handoff_kind),
                    title: block.child.title.clone(),
                    lifecycle: block.child.lifecycle,
                    message: block
                        .message
                        .clone()
                        .unwrap_or_else(|| "无摘要".to_string()),
                    child_session_id: block.child.child_session_id.clone(),
                    child_agent_id: block.child.child_agent_id.clone(),
                },
            },
        }
    }
}

impl From<AstrcodeConversationBlockStatusDto> for TranscriptCellStatus {
    fn from(value: AstrcodeConversationBlockStatusDto) -> Self {
        match value {
            AstrcodeConversationBlockStatusDto::Streaming => Self::Streaming,
            AstrcodeConversationBlockStatusDto::Complete => Self::Complete,
            AstrcodeConversationBlockStatusDto::Failed => Self::Failed,
            AstrcodeConversationBlockStatusDto::Cancelled => Self::Cancelled,
        }
    }
}
