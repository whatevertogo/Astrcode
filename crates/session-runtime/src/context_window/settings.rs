use std::time::Duration;

use astrcode_core::ResolvedRuntimeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindowSettings {
    pub auto_compact_enabled: bool,
    pub compact_threshold_percent: u8,
    pub tool_result_max_bytes: usize,
    pub compact_keep_recent_turns: usize,
    pub max_tracked_files: usize,
    pub max_recovered_files: usize,
    pub recovery_token_budget: usize,
    pub aggregate_result_bytes_budget: usize,
    pub micro_compact_gap_threshold: Duration,
    pub micro_compact_keep_recent_results: usize,
}

impl ContextWindowSettings {
    pub fn micro_compact_config(&self) -> crate::context_window::micro_compact::MicroCompactConfig {
        crate::context_window::micro_compact::MicroCompactConfig {
            gap_threshold: self.micro_compact_gap_threshold,
            keep_recent_results: self.micro_compact_keep_recent_results,
        }
    }

    pub fn file_recovery_config(&self) -> crate::context_window::file_access::FileRecoveryConfig {
        crate::context_window::file_access::FileRecoveryConfig {
            max_tracked_files: self.max_tracked_files,
            max_recovered_files: self.max_recovered_files,
            recovery_token_budget: self.recovery_token_budget,
        }
    }
}

impl From<&ResolvedRuntimeConfig> for ContextWindowSettings {
    fn from(config: &ResolvedRuntimeConfig) -> Self {
        Self {
            auto_compact_enabled: config.auto_compact_enabled,
            compact_threshold_percent: config.compact_threshold_percent,
            tool_result_max_bytes: config.tool_result_max_bytes,
            compact_keep_recent_turns: usize::from(config.compact_keep_recent_turns),
            max_tracked_files: config.max_tracked_files,
            max_recovered_files: config.max_recovered_files.max(1),
            recovery_token_budget: config.recovery_token_budget.max(1),
            aggregate_result_bytes_budget: config.aggregate_result_bytes_budget.max(1),
            micro_compact_gap_threshold: Duration::from_secs(
                config.micro_compact_gap_threshold_secs.max(1),
            ),
            micro_compact_keep_recent_results: config.micro_compact_keep_recent_results.max(1),
        }
    }
}
