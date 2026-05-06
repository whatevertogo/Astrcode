use std::{fmt, path::PathBuf, sync::Arc};

use astrcode_context_window::tool_result_budget::ToolResultReplacementRecord;
use astrcode_core::{
    AgentEventContext, CapabilitySpec, LlmMessage, ResolvedRuntimeConfig, TurnTerminalKind,
    llm::LlmProvider,
};
use astrcode_runtime_contract::{RuntimeEventSink, RuntimeTurnEvent, TurnIdentity, TurnStopCause};
use chrono::{DateTime, Utc};

use crate::{hook_dispatch::HookDispatcher, tool_dispatch::ToolDispatcher};

/// `host-session -> agent-runtime` 的最小执行面骨架。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentRuntimeExecutionSurface {
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
    pub model_ref: String,
    pub provider_ref: String,
    pub tool_specs: Vec<CapabilitySpec>,
    pub hook_snapshot_id: String,
    pub current_mode: Option<String>,
}

#[derive(Clone, Default)]
pub struct TurnInput {
    pub surface: AgentRuntimeExecutionSurface,
    /// Stable event metadata supplied by host-session.
    ///
    /// The runtime forwards this context to hook/tool execution surfaces, but it
    /// must not derive collaboration truth, parent/child linkage, or input queue
    /// state from it.
    pub agent: AgentEventContext,
    pub messages: Vec<LlmMessage>,
    pub provider: Option<Arc<dyn LlmProvider>>,
    pub tool_dispatcher: Option<Arc<dyn ToolDispatcher>>,
    pub hook_dispatcher: Option<Arc<dyn HookDispatcher>>,
    pub cancel: astrcode_core::CancelToken,
    pub event_sink: Option<Arc<dyn RuntimeEventSink>>,
    pub max_output_continuations: usize,
    pub working_dir: PathBuf,
    pub runtime_config: ResolvedRuntimeConfig,
    pub last_assistant_at: Option<DateTime<Utc>>,
    pub previous_tool_result_replacements: Vec<ToolResultReplacementRecord>,
    /// 事件历史 JSONL 文件路径（如 `{project_dir}/sessions/{session_id}/events.jsonl`）。
    ///
    /// 由宿主提供，`agent-runtime` 自身不构造路径。`None` 表示不保存 compact 历史。
    pub events_history_path: Option<String>,
}

impl fmt::Debug for TurnInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TurnInput")
            .field("surface", &self.surface)
            .field("agent", &self.agent)
            .field("messages", &self.messages)
            .field(
                "provider",
                &self.provider.as_ref().map(|_| "<llm-provider>"),
            )
            .field(
                "tool_dispatcher",
                &self.tool_dispatcher.as_ref().map(|_| "<tool-dispatcher>"),
            )
            .field(
                "hook_dispatcher",
                &self.hook_dispatcher.as_ref().map(|_| "<hook-dispatcher>"),
            )
            .field("cancel", &self.cancel)
            .field(
                "event_sink",
                &self.event_sink.as_ref().map(|_| "<runtime-event-sink>"),
            )
            .field("max_output_continuations", &self.max_output_continuations)
            .field("working_dir", &self.working_dir)
            .field("runtime_config", &self.runtime_config)
            .field("last_assistant_at", &self.last_assistant_at)
            .field(
                "previous_tool_result_replacements",
                &self.previous_tool_result_replacements,
            )
            .field("events_history_path", &self.events_history_path)
            .finish()
    }
}

impl TurnInput {
    pub fn new(surface: AgentRuntimeExecutionSurface) -> Self {
        Self {
            surface,
            agent: AgentEventContext::default(),
            messages: Vec::new(),
            provider: None,
            tool_dispatcher: None,
            hook_dispatcher: None,
            cancel: astrcode_core::CancelToken::new(),
            event_sink: None,
            max_output_continuations: 0,
            working_dir: PathBuf::new(),
            runtime_config: ResolvedRuntimeConfig::default(),
            last_assistant_at: None,
            previous_tool_result_replacements: Vec::new(),
            events_history_path: None,
        }
    }

    pub fn with_messages(mut self, messages: Vec<LlmMessage>) -> Self {
        self.messages = messages;
        self
    }

    pub fn with_agent(mut self, agent: AgentEventContext) -> Self {
        self.agent = agent;
        self
    }

    pub fn with_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_tool_dispatcher(mut self, tool_dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(tool_dispatcher);
        self
    }

    pub fn with_hook_dispatcher(mut self, hook_dispatcher: Arc<dyn HookDispatcher>) -> Self {
        self.hook_dispatcher = Some(hook_dispatcher);
        self
    }

    pub fn with_cancel(mut self, cancel: astrcode_core::CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn with_event_sink(mut self, event_sink: Arc<dyn RuntimeEventSink>) -> Self {
        self.event_sink = Some(event_sink);
        self
    }

    pub fn with_max_output_continuations(mut self, max_output_continuations: usize) -> Self {
        self.max_output_continuations = max_output_continuations;
        self
    }

    pub fn with_working_dir(mut self, working_dir: impl Into<PathBuf>) -> Self {
        self.working_dir = working_dir.into();
        self
    }

    pub fn with_runtime_config(mut self, runtime_config: ResolvedRuntimeConfig) -> Self {
        self.runtime_config = runtime_config;
        self
    }

    pub fn with_last_assistant_at(mut self, last_assistant_at: Option<DateTime<Utc>>) -> Self {
        self.last_assistant_at = last_assistant_at;
        self
    }

    pub fn with_previous_tool_result_replacements(
        mut self,
        replacements: Vec<ToolResultReplacementRecord>,
    ) -> Self {
        self.previous_tool_result_replacements = replacements;
        self
    }

    pub fn with_events_history_path(mut self, path: Option<String>) -> Self {
        self.events_history_path = path;
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct TurnOutput {
    pub identity: TurnIdentity,
    pub terminal_kind: Option<TurnTerminalKind>,
    pub stop_cause: Option<TurnStopCause>,
    pub step_count: usize,
    pub events: Vec<RuntimeTurnEvent>,
    pub error_message: Option<String>,
}

impl TurnOutput {
    pub fn empty_for(input: TurnInput) -> Self {
        let identity = TurnIdentity::new(
            input.surface.session_id,
            input.surface.turn_id,
            input.surface.agent_id,
        );
        Self {
            identity,
            terminal_kind: None,
            stop_cause: None,
            step_count: 0,
            events: Vec::new(),
            error_message: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEventContext, SubRunStorageMode, TurnTerminalKind};
    use astrcode_runtime_contract::TurnStopCause;

    use super::{AgentRuntimeExecutionSurface, TurnInput, TurnOutput};

    #[test]
    fn empty_output_keeps_turn_identity() {
        let input = TurnInput::new(AgentRuntimeExecutionSurface {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            agent_id: "agent-1".to_string(),
            model_ref: "model-a".to_string(),
            provider_ref: "provider-a".to_string(),
            tool_specs: Vec::new(),
            hook_snapshot_id: "snapshot-1".to_string(),
            current_mode: None,
        });

        let output = TurnOutput::empty_for(input);

        assert_eq!(output.identity.session_id, "session-1");
        assert_eq!(output.identity.turn_id, "turn-1");
        assert_eq!(output.identity.agent_id, "agent-1");
    }

    #[test]
    fn turn_input_carries_agent_event_context_without_collaboration_state() {
        let agent = AgentEventContext::sub_run(
            "agent-child",
            "parent-turn",
            "default",
            "subrun-1",
            None,
            SubRunStorageMode::IndependentSession,
            Some("child-session-1".to_string().into()),
        );
        let input = TurnInput::new(AgentRuntimeExecutionSurface {
            session_id: "child-session-1".to_string(),
            turn_id: "child-turn-1".to_string(),
            agent_id: "agent-child".to_string(),
            model_ref: "model-a".to_string(),
            provider_ref: "provider-a".to_string(),
            tool_specs: Vec::new(),
            hook_snapshot_id: "snapshot-1".to_string(),
            current_mode: None,
        })
        .with_agent(agent.clone());

        assert_eq!(input.agent, agent);
        assert!(input.agent.is_independent_sub_run());
        assert!(input.agent.belongs_to_child_session("child-session-1"));
    }

    #[test]
    fn stop_cause_maps_terminal_kind() {
        assert_eq!(
            TurnStopCause::Completed.terminal_kind(None),
            TurnTerminalKind::Completed
        );
        assert_eq!(
            TurnStopCause::Cancelled.terminal_kind(None),
            TurnTerminalKind::Cancelled
        );
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
}
