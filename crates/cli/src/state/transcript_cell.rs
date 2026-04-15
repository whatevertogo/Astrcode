use astrcode_client::{
    AstrcodeConversationAgentLifecycleDto, AstrcodeConversationBlockDto,
    AstrcodeConversationBlockStatusDto,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptCell {
    pub id: String,
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
    },
    ToolStream {
        stream: String,
        content: String,
        status: TranscriptCellStatus,
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
    pub fn from_block(block: &AstrcodeConversationBlockDto) -> Self {
        match block {
            AstrcodeConversationBlockDto::User(block) => Self {
                id: block.id.clone(),
                kind: TranscriptCellKind::User {
                    body: block.markdown.clone(),
                },
            },
            AstrcodeConversationBlockDto::Assistant(block) => Self {
                id: block.id.clone(),
                kind: TranscriptCellKind::Assistant {
                    body: block.markdown.clone(),
                    status: block.status.into(),
                },
            },
            AstrcodeConversationBlockDto::Thinking(block) => Self {
                id: block.id.clone(),
                kind: TranscriptCellKind::Thinking {
                    body: block.markdown.clone(),
                    status: block.status.into(),
                },
            },
            AstrcodeConversationBlockDto::ToolCall(block) => Self {
                id: block.id.clone(),
                kind: TranscriptCellKind::ToolCall {
                    tool_name: block.tool_name.clone(),
                    summary: block
                        .summary
                        .clone()
                        .unwrap_or_else(|| "正在执行工具调用".to_string()),
                    status: block.status.into(),
                },
            },
            AstrcodeConversationBlockDto::ToolStream(block) => Self {
                id: block.id.clone(),
                kind: TranscriptCellKind::ToolStream {
                    stream: format!("{:?}", block.stream),
                    content: block.content.clone(),
                    status: block.status.into(),
                },
            },
            AstrcodeConversationBlockDto::Error(block) => Self {
                id: block.id.clone(),
                kind: TranscriptCellKind::Error {
                    code: format!("{:?}", block.code),
                    message: block.message.clone(),
                },
            },
            AstrcodeConversationBlockDto::SystemNote(block) => Self {
                id: block.id.clone(),
                kind: TranscriptCellKind::SystemNote {
                    note_kind: format!("{:?}", block.note_kind),
                    markdown: block.markdown.clone(),
                },
            },
            AstrcodeConversationBlockDto::ChildHandoff(block) => Self {
                id: block.id.clone(),
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
