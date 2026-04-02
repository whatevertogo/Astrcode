//! Shared context-window management for prompt sizing and compaction.
//!
//! This module is intentionally independent from `agent_loop` so both the
//! online turn runner and service-level manual compact flow use the same
//! algorithms and data contracts.

pub(crate) mod compaction;
pub(crate) mod microcompact;
pub(crate) mod token_usage;

pub(crate) use compaction::{auto_compact, CompactConfig};
pub(crate) use microcompact::apply_microcompact;
pub(crate) use token_usage::{
    build_prompt_snapshot, effective_context_window, estimate_message_tokens,
    estimate_request_tokens, estimate_text_tokens, should_compact, TokenUsageTracker,
};
