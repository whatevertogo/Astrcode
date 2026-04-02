//! # 服务支持工具 (Service Support Utilities)
//!
//! 提供 `RuntimeService` 内部使用的辅助函数：
//! - `lock_anyhow` - 将 `std::sync::Mutex` 的 PoisonError 转换为 anyhow 错误
//! - `spawn_blocking_anyhow` - 在阻塞线程池上运行返回 `anyhow::Result` 的工作
//! - `spawn_blocking_service` - 在阻塞线程池上运行返回 `ServiceResult` 的工作
//!
//! ## 为什么需要 spawn_blocking_service
//!
//! `RuntimeService` 中很多操作（如文件 I/O、配置解析）是阻塞的，
//! 但服务本身是异步的。`spawn_blocking_service` 桥接了这个差距：
//! 1. 将工作提交到 Tokio 的阻塞线程池
//! 2. 保留 `ServiceError` 的原始变体（通过 anyhow 包装）
//! 3. 在异步边界后恢复原始错误类型
//!
//! 这避免了每个调用点都重复写 `spawn_blocking` + 错误映射的样板代码。

use std::sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard};

use anyhow::Result;
use astrcode_core::AstrError;

use super::{ServiceError, ServiceResult};

pub(super) fn lock_anyhow<'a, T>(
    mutex: &'a StdMutex<T>,
    name: &'static str,
) -> Result<StdMutexGuard<'a, T>> {
    Ok(mutex
        .lock()
        .map_err(|_| AstrError::LockPoisoned(name.to_string()))?)
}

pub(super) async fn spawn_blocking_anyhow<T, F>(label: &'static str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| AstrError::Internal(format!("blocking task '{label}' failed: {error}")))?
}

/// Bridge helper: runs blocking work that returns [`ServiceResult`] and flattens.
///
/// This avoids duplicating the `spawn_blocking` + error-mapping boilerplate
/// in every call site that still uses `ServiceResult`.
pub(super) async fn spawn_blocking_service<T, F>(label: &'static str, work: F) -> ServiceResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> ServiceResult<T> + Send + 'static,
{
    spawn_blocking_anyhow(label, move || {
        // Preserve the original ServiceError inside anyhow so the async boundary can
        // recover the exact variant instead of degrading everything into Internal(...).
        work().map_err(anyhow::Error::new)
    })
    .await
    .map_err(ServiceError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_blocking_service_preserves_service_error_variants() {
        let error = spawn_blocking_service::<(), _>("preserve service error", || {
            Err(ServiceError::NotFound("missing session".to_string()))
        })
        .await
        .expect_err("service error should bubble through blocking bridge");

        assert!(matches!(error, ServiceError::NotFound(message) if message == "missing session"));
    }
}
