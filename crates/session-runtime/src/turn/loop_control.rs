//! turn loop 的显式过渡/停止语义。
//!
//! Why: `request -> llm -> tool` 的编排已经模块化，但“为什么继续/停止”
//! 仍需要一个稳定骨架，否则后续输出截断恢复和流式工具调度
//! 都会退化成新的局部布尔值。

use astrcode_core::TurnTerminalKind;

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
    StepLimitExceeded,
    MaxOutputContinuationLimitReached,
}

impl TurnStopCause {
    pub fn legacy_turn_done_reason(self) -> Option<&'static str> {
        match self {
            Self::Completed => Some("completed"),
            Self::MaxOutputContinuationLimitReached => Some("token_exceeded"),
            Self::Cancelled | Self::Error | Self::StepLimitExceeded => None,
        }
    }

    pub fn terminal_kind(self, error_message: Option<&str>) -> TurnTerminalKind {
        match self {
            Self::Completed => TurnTerminalKind::Completed,
            Self::Cancelled => TurnTerminalKind::Cancelled,
            Self::Error => TurnTerminalKind::Error {
                message: error_message.unwrap_or("turn failed").to_string(),
            },
            Self::StepLimitExceeded => TurnTerminalKind::StepLimitExceeded,
            Self::MaxOutputContinuationLimitReached => {
                TurnTerminalKind::MaxOutputContinuationLimitReached
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::TurnTerminalKind;

    use super::*;

    #[test]
    fn error_stop_cause_maps_to_error_terminal_kind() {
        assert_eq!(
            TurnStopCause::Error.terminal_kind(Some("boom")),
            TurnTerminalKind::Error {
                message: "boom".to_string()
            }
        );
        assert_eq!(
            TurnStopCause::Error.terminal_kind(None),
            TurnTerminalKind::Error {
                message: "turn failed".to_string()
            }
        );
    }

    #[test]
    fn max_output_stop_cause_maps_to_token_exceeded_reason() {
        assert_eq!(
            TurnStopCause::MaxOutputContinuationLimitReached.legacy_turn_done_reason(),
            Some("token_exceeded")
        );
    }
}
