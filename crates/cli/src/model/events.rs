#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventLog {
    entries: Vec<Event>,
}

impl EventLog {
    pub fn new(entries: Vec<Event>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[Event] {
        &self.entries
    }

    pub fn replace(&mut self, entries: Vec<Event>) {
        self.entries = entries;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    UserTurn {
        id: String,
        text: String,
    },
    AssistantBlock {
        id: String,
        text: String,
        streaming: bool,
    },
    Thinking {
        id: String,
        summary: String,
        preview: String,
    },
    ToolStatus {
        id: String,
        tool_name: String,
        summary: String,
    },
    ToolSummary {
        id: String,
        tool_name: String,
        summary: String,
        artifact_path: Option<String>,
    },
    SystemNote {
        id: String,
        text: String,
    },
    Error {
        id: String,
        text: String,
    },
}
