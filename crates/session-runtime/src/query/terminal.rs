use astrcode_core::{CompactAppliedMeta, CompactTrigger, Phase};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastCompactMetaSnapshot {
    pub trigger: CompactTrigger,
    pub meta: CompactAppliedMeta,
}

/// terminal / interactive surface 需要的稳定控制态快照。
///
/// Why: application 只应消费可序列化、可测试的读模型事实，
/// 不能透过 `SessionState` 直接读取内部 mutex 字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionControlStateSnapshot {
    pub phase: Phase,
    pub active_turn_id: Option<String>,
    pub manual_compact_pending: bool,
    pub compacting: bool,
    pub last_compact_meta: Option<LastCompactMetaSnapshot>,
    pub current_mode_id: astrcode_core::ModeId,
    pub last_mode_changed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionModeSnapshot {
    pub current_mode_id: astrcode_core::ModeId,
    pub last_mode_changed_at: Option<DateTime<Utc>>,
}
