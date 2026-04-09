use std::sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard};

use anyhow::Result;
use astrcode_core::AstrError;

pub fn lock_anyhow<'a, T>(
    mutex: &'a StdMutex<T>,
    name: &'static str,
) -> Result<StdMutexGuard<'a, T>> {
    mutex.lock().map_err(|_| {
        log::error!("mutex '{name}' is poisoned; refusing to continue with stale session state");
        AstrError::LockPoisoned(name.to_string()).into()
    })
}

pub fn with_lock_recovery<T, R>(
    mutex: &StdMutex<T>,
    name: &'static str,
    update: impl FnOnce(&mut T) -> R,
) -> R {
    match mutex.lock() {
        Ok(mut guard) => update(&mut guard),
        Err(poisoned) => {
            log::error!("mutex '{name}' was poisoned; recovering inner state");
            let mut guard = poisoned.into_inner();
            let result = update(&mut guard);
            mutex.clear_poison();
            result
        },
    }
}

pub async fn spawn_blocking_anyhow<T, F>(label: &'static str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        log::error!("blocking task '{label}' failed before returning a session result: {error}");
        AstrError::Internal(format!("blocking task '{label}' failed: {error}"))
    })?
}
