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
