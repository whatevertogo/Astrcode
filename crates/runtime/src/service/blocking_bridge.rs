//! # 服务阻塞桥接 (Service Blocking Bridge)
//!
//! 提供 `RuntimeService` 内部使用的阻塞桥接工具：
//! - `lock_anyhow`：复用 `runtime-session` 的实现，把 `std::sync::Mutex` 的毒化错误转换成领域错误
//! - `spawn_blocking_anyhow`：复用 `runtime-session` 的实现，在阻塞线程池中执行返回
//!   `anyhow::Result` 的任务
//! - `spawn_blocking_service`：在阻塞线程池中执行返回 `ServiceResult` 的任务（runtime 特有）
//!
//! 这些辅助只负责跨越 async/blocking 边界，不承载具体业务语义。

// 复用 runtime-session 的通用实现，避免重复定义
pub(super) use astrcode_runtime_session::{lock_anyhow, spawn_blocking_anyhow};

use super::{ServiceError, ServiceResult};

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
