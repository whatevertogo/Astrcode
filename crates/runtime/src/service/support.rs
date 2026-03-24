use std::sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard};

use anyhow::Result;
use astrcode_core::AstrError;

use super::{ServiceError, ServiceResult};

pub(super) fn lock_service<'a, T>(
    mutex: &'a StdMutex<T>,
    name: &'static str,
) -> ServiceResult<StdMutexGuard<'a, T>> {
    mutex
        .lock()
        .map_err(|_| ServiceError::Internal(AstrError::LockPoisoned(name.to_string())))
}

pub(super) fn lock_anyhow<'a, T>(
    mutex: &'a StdMutex<T>,
    name: &'static str,
) -> Result<StdMutexGuard<'a, T>> {
    Ok(mutex
        .lock()
        .map_err(|_| AstrError::LockPoisoned(name.to_string()))?)
}

pub(super) async fn spawn_blocking_service<T, F>(label: &'static str, work: F) -> ServiceResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> ServiceResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        ServiceError::Internal(AstrError::Internal(format!(
            "blocking task '{label}' failed: {error}"
        )))
    })?
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
