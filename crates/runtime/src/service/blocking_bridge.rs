//! # 服务阻塞桥接 (Service Blocking Bridge)
//!
//! 提供 `RuntimeService` 内部使用的阻塞桥接工具：
//! - `lock_anyhow`：把 `std::sync::Mutex` 的毒化错误转换成领域错误
//! - `spawn_blocking_anyhow`：在阻塞线程池中执行返回 `anyhow::Result` 的任务
//! - `spawn_blocking_service`：在阻塞线程池中执行返回 `ServiceResult` 的任务
//!
//! 这些辅助只负责跨越 async/blocking 边界，不承载具体业务语义。

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

/// 阻塞桥接只负责保留 `ServiceError` 原始变体，避免 async 边界把它们全部抹平成
/// `Internal(...)`，这样 HTTP 层还能继续拿到正确的 404 / 409 / 400 语义。
pub(super) async fn spawn_blocking_service<T, F>(label: &'static str, work: F) -> ServiceResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> ServiceResult<T> + Send + 'static,
{
    spawn_blocking_anyhow(label, move || work().map_err(anyhow::Error::new))
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
