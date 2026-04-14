use std::sync::{Arc, Mutex as StdMutex};

use astrcode_core::{EventLogWriter, Result, StorageEvent, StoredEvent, support};

/// 同步 `EventLogWriter` 的 async-safe 包装。
///
/// `EventLogWriter` 是同步 trait，但 session-runtime 运行在 tokio 异步上下文中，
/// 这里用 `StdMutex` 保护内部 writer，并通过 `spawn_blocking` 桥接到异步调用。
pub struct SessionWriter {
    inner: StdMutex<Box<dyn EventLogWriter>>,
}

impl SessionWriter {
    pub fn new(writer: Box<dyn EventLogWriter>) -> Self {
        Self {
            inner: StdMutex::new(writer),
        }
    }

    /// 同步写入：在当前线程直接调用 writer，用于 `spawn_blocking` 内部或测试。
    pub fn append_blocking(&self, event: &StorageEvent) -> Result<StoredEvent> {
        event.validate()?;
        let mut guard = support::lock_anyhow(&self.inner, "session writer")?;
        guard.append(event).map_err(|error| {
            astrcode_core::AstrError::Internal(format!("session write failed: {error}"))
        })
    }

    /// 异步写入：通过 `spawn_blocking` 在专用线程池执行同步 I/O，避免阻塞 tokio runtime。
    pub async fn append(self: Arc<Self>, event: StorageEvent) -> Result<StoredEvent> {
        spawn_blocking_result("append session event", move || self.append_blocking(&event)).await
    }
}

/// 将同步闭包包装为 `spawn_blocking` 异步调用，统一处理 JoinError。
async fn spawn_blocking_result<T, F>(label: &'static str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        log::error!("blocking task '{label}' failed: {error}");
        astrcode_core::AstrError::Internal(format!("blocking task '{label}' failed: {error}"))
    })?
}
