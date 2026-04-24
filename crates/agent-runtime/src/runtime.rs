use crate::{
    r#loop::TurnLoop,
    types::{TurnInput, TurnOutput},
};

/// 最小执行入口。
#[derive(Debug, Default)]
pub struct AgentRuntime;

impl AgentRuntime {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute_turn(&self, input: TurnInput) -> TurnOutput {
        TurnLoop.run(input).await
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::TurnTerminalKind;
    use astrcode_runtime_contract::TurnStopCause;

    use super::AgentRuntime;
    use crate::types::{AgentRuntimeExecutionSurface, TurnInput};

    #[tokio::test]
    async fn execute_turn_drives_empty_lifecycle() {
        let input = TurnInput::new(AgentRuntimeExecutionSurface {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            agent_id: "agent-1".to_string(),
            model_ref: "model-a".to_string(),
            provider_ref: "provider-a".to_string(),
            tool_specs: Vec::new(),
            hook_snapshot_id: "snapshot-1".to_string(),
        });

        let output = AgentRuntime::new().execute_turn(input).await;

        assert_eq!(output.stop_cause, Some(TurnStopCause::Completed));
        assert_eq!(output.terminal_kind, Some(TurnTerminalKind::Completed));
        assert_eq!(output.step_count, 1);
        assert_eq!(output.events.len(), 2);
    }
}
