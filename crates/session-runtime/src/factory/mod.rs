//! 单 session 执行对象构造辅助。
//!
//! Why: `factory` 只保留“构造执行对象”这类无状态职责，
//! 状态读取应通过 `query` 完成，避免 factory 膨胀成杂项入口。

use astrcode_core::SessionTurnLease;

#[derive(Debug)]
pub struct NoopSessionTurnLease;

impl SessionTurnLease for NoopSessionTurnLease {}
