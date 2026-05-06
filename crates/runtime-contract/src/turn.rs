use astrcode_core::{
    AstrError, HookEventKey, ReasoningContent, StorageEvent, TurnTerminalKind, llm::LlmEvent,
};

/// runtime 事件发射回调。
///
/// `agent-runtime` 只通过这个回调把 turn 生命周期事件交还给宿主，不持有
/// EventStore、SessionState 或 plugin registry。
pub trait RuntimeEventSink: Send + Sync {
    fn emit_event(&self, event: RuntimeTurnEvent);
}

impl<F> RuntimeEventSink for F
where
    F: Fn(RuntimeTurnEvent) + Send + Sync,
{
    fn emit_event(&self, event: RuntimeTurnEvent) {
        self(event);
    }
}

/// 内部 loop 的“继续下一轮”原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnLoopTransition {
    ToolCycleCompleted,
    ReactiveCompactRecovered,
    OutputContinuationRequested,
}

/// turn 停止的细粒度原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStopCause {
    Completed,
    Cancelled,
    Error,
}

impl TurnStopCause {
    pub fn terminal_kind(self, error_message: Option<&str>) -> TurnTerminalKind {
        match self {
            Self::Completed => TurnTerminalKind::Completed,
            Self::Cancelled => TurnTerminalKind::Cancelled,
            Self::Error => TurnTerminalKind::Error {
                message: error_message.unwrap_or("turn failed").to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnIdentity {
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
}

impl TurnIdentity {
    pub fn new(session_id: String, turn_id: String, agent_id: String) -> Self {
        Self {
            session_id,
            turn_id,
            agent_id,
        }
    }
}

/// 单步执行中产生的错误，保留可重试/致命区分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepError {
    pub message: String,
    pub kind: StepErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepErrorKind {
    Fatal,
    Retryable,
}

impl StepError {
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StepErrorKind::Fatal,
        }
    }

    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: StepErrorKind::Retryable,
        }
    }
}

impl From<&AstrError> for StepError {
    fn from(error: &AstrError) -> Self {
        Self {
            message: error.to_string(),
            kind: if error.is_retryable() {
                StepErrorKind::Retryable
            } else {
                StepErrorKind::Fatal
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum RuntimeTurnEvent {
    TurnStarted {
        identity: TurnIdentity,
    },
    ProviderStream {
        identity: TurnIdentity,
        event: LlmEvent,
    },
    AssistantFinal {
        identity: TurnIdentity,
        content: String,
        reasoning: Option<ReasoningContent>,
        tool_call_count: usize,
    },
    ToolUseRequested {
        identity: TurnIdentity,
        tool_call_count: usize,
    },
    ToolCallStarted {
        identity: TurnIdentity,
        tool_call_id: String,
        tool_name: String,
    },
    ToolResultReady {
        identity: TurnIdentity,
        tool_call_id: String,
        tool_name: String,
        ok: bool,
    },
    HookDispatched {
        identity: TurnIdentity,
        event: HookEventKey,
        effect_count: usize,
    },
    HookPromptAugmented {
        identity: TurnIdentity,
        event: HookEventKey,
        content: String,
    },
    StorageEvent {
        event: Box<StorageEvent>,
    },
    StepContinued {
        identity: TurnIdentity,
        step_index: usize,
        transition: TurnLoopTransition,
    },
    TurnCompleted {
        identity: TurnIdentity,
        stop_cause: TurnStopCause,
        terminal_kind: TurnTerminalKind,
    },
    TurnErrored {
        identity: TurnIdentity,
        message: String,
    },
}
