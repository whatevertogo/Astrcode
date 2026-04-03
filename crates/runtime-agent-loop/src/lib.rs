//! Agent loop execution engine and shared context-window algorithms.

pub mod agent_loop;
pub mod approval_service;
pub mod context_window;
pub mod provider_factory;
#[cfg(test)]
mod test_support;

pub use agent_loop::token_budget::{
    build_auto_continue_nudge, check_token_budget, strip_token_budget_marker, TokenBudgetDecision,
};
pub use agent_loop::{AgentLoop, TurnOutcome};
pub use approval_service::{ApprovalBroker, DefaultApprovalBroker};
pub use context_window::{
    auto_compact, build_prompt_snapshot, effective_context_window, estimate_message_tokens,
    estimate_request_tokens, estimate_text_tokens, is_prompt_too_long, should_compact,
    CompactConfig, CompactResult, PromptTokenSnapshot, TokenUsageTracker,
};
pub use provider_factory::{DynProviderFactory, ProviderFactory};
