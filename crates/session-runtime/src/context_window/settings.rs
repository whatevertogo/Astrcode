use std::time::Duration;

use astrcode_core::config::RuntimeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindowSettings {
    pub auto_compact_enabled: bool,
    pub compact_threshold_percent: u8,
    pub tool_result_max_bytes: usize,
    pub compact_keep_recent_turns: usize,
    pub max_tracked_files: usize,
    pub max_recovered_files: usize,
    pub recovery_token_budget: usize,
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

impl From<&RuntimeConfig> for ContextWindowSettings {
    fn from(config: &RuntimeConfig) -> Self {
        const DEFAULT_AUTO_COMPACT_ENABLED: bool = true;
        const DEFAULT_COMPACT_THRESHOLD_PERCENT: u8 = 90;
        const DEFAULT_TOOL_RESULT_MAX_BYTES: usize = 100_000;
        const DEFAULT_COMPACT_KEEP_RECENT_TURNS: usize = 2;
        const DEFAULT_MAX_TRACKED_FILES: usize = 12;
        const DEFAULT_MAX_RECOVERED_FILES: usize = 3;
        const DEFAULT_RECOVERY_TOKEN_BUDGET: usize = 6_000;
        const DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS: u64 = 45;
        const DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS: usize = 2;

        Self {
            auto_compact_enabled: config
                .auto_compact_enabled
                .unwrap_or(DEFAULT_AUTO_COMPACT_ENABLED),
            compact_threshold_percent: config
                .compact_threshold_percent
                .unwrap_or(DEFAULT_COMPACT_THRESHOLD_PERCENT),
            tool_result_max_bytes: config
                .tool_result_max_bytes
                .unwrap_or(DEFAULT_TOOL_RESULT_MAX_BYTES),
            compact_keep_recent_turns: config
                .compact_keep_recent_turns
                .map(usize::from)
                .unwrap_or(DEFAULT_COMPACT_KEEP_RECENT_TURNS)
                .max(1),
            max_tracked_files: config
                .max_tracked_files
                .unwrap_or(DEFAULT_MAX_TRACKED_FILES)
                .max(1),
            max_recovered_files: config
                .max_recovered_files
                .unwrap_or(DEFAULT_MAX_RECOVERED_FILES)
                .max(1),
            recovery_token_budget: config
                .recovery_token_budget
                .unwrap_or(DEFAULT_RECOVERY_TOKEN_BUDGET)
                .max(1),
            micro_compact_gap_threshold: Duration::from_secs(
                config
                    .micro_compact_gap_threshold_secs
                    .unwrap_or(DEFAULT_MICRO_COMPACT_GAP_THRESHOLD_SECS)
                    .max(1),
            ),
            micro_compact_keep_recent_results: config
                .micro_compact_keep_recent_results
                .unwrap_or(DEFAULT_MICRO_COMPACT_KEEP_RECENT_RESULTS)
                .max(1),
        }
    }
}
