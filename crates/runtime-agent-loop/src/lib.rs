//! Agent loop execution engine and shared context-window algorithms.

pub mod agent_loop;
pub mod approval_service;
mod compaction_runtime;
mod context_pipeline;
pub mod context_window;
mod prompt_runtime;
pub mod provider_factory;
mod request_assembler;

pub use agent_loop::{
    AgentLoop, TurnOutcome,
    token_budget::{
        TokenBudgetDecision, build_auto_continue_nudge, check_token_budget,
        strip_token_budget_marker,
    },
};
pub use approval_service::{ApprovalBroker, DefaultApprovalBroker};
pub use compaction_runtime::CompactionTailSnapshot;
pub use context_window::{
    CompactConfig, CompactResult, PromptTokenSnapshot, TokenUsageTracker, auto_compact,
    build_prompt_snapshot, effective_context_window, estimate_message_tokens,
    estimate_request_tokens, estimate_text_tokens, is_prompt_too_long, should_compact,
};
pub use provider_factory::{DynProviderFactory, ProviderFactory};
