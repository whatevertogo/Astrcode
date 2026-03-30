//! # 会话写入器
//!
//! 线程安全的会话事件写入器。

use std::sync::Mutex;

use crate::{EventLog, Result, StorageEvent, StoredEvent};

/// 会话写入器
///
/// 使用 `Mutex<EventLog>` 提供线程安全的事件追加能力。
/// 适用于多线程场景（如 SSE 广播时同时写入事件）。
pub struct SessionWriter {
    /// 内部事件日志，使用 Mutex 保护
    inner: Mutex<EventLog>,
}

impl SessionWriter {
    /// 创建新的会话写入器
    pub fn new(log: EventLog) -> Self {
        Self {
            inner: Mutex::new(log),
        }
    }

    /// 阻塞式追加事件
    ///
    /// 获取锁后会阻塞直到写入完成。
    /// 如果锁被污染（其他线程 panic），返回错误。
    pub fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| crate::AstrError::LockPoisoned("session writer".to_string()))?;
        guard.append(event)
    }
}
