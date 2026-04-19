//! # 终端查询子域
//!
//! 从旧 `terminal_use_cases.rs` 拆分而来，按职责分为四个查询模块：
//! - `cursor`：游标格式校验与比较
//! - `resume`：会话恢复候选列表
//! - `snapshot`：会话快照查询（conversation + transcript）
//! - `summary`：会话摘要提取

mod cursor;
mod resume;
mod snapshot;
mod summary;
#[cfg(test)]
mod tests;

use astrcode_session_runtime::SessionControlStateSnapshot;

use crate::terminal::{ActivePlanFacts, TerminalControlFacts, TerminalLastCompactMetaFacts};

fn map_control_facts(control: SessionControlStateSnapshot) -> TerminalControlFacts {
    TerminalControlFacts {
        phase: control.phase,
        active_turn_id: control.active_turn_id,
        manual_compact_pending: control.manual_compact_pending,
        compacting: control.compacting,
        last_compact_meta: control
            .last_compact_meta
            .map(|meta| TerminalLastCompactMetaFacts {
                trigger: meta.trigger,
                meta: meta.meta,
            }),
        active_plan: None::<ActivePlanFacts>,
    }
}
