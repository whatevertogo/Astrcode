//! Context window management.
//!
//! It owns token budgeting and message trimming/compaction operations:
//! - `token_usage`: token estimation, budget tracking and compaction threshold metrics
//! - `prune_pass`: lightweight truncation of clearable tool results (no LLM)
//! - `compaction`: context compaction (LLM-required summarization)
//! - `micro_compact`: idle-time cleanup of stale tool-result traces
//! - `file_access`: replaying recovered file-context messages
//! - `settings`: window/compaction parameter mapping
//!
//! Final request assembly must not be implemented here.
//! That flow is implemented in `session-runtime::turn::request`.

pub(crate) mod compaction;
pub(crate) mod file_access;
pub(crate) mod micro_compact;
pub(crate) mod prune_pass;
pub(crate) mod settings;
pub(crate) mod token_usage;
pub(crate) mod tool_results;

pub(crate) use settings::ContextWindowSettings;
