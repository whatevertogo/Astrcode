//! 通用锁安全获取工具。
//!
//! 提供 `with_lock_recovery` 和 `lock_anyhow` 函数，
//! 用于在 StdMutex 中毒时恢复内部状态而非 panic。
//! 与 `runtime-session/src/support.rs` 中的同名函数行为一致，
//! 统一放至 core 层以便所有 crate 复用。

use std::sync::{
    Mutex as StdMutex, MutexGuard as StdMutexGuard, RwLock as StdRwLock,
    RwLockReadGuard as StdRwLockReadGuard, RwLockWriteGuard as StdRwLockWriteGuard,
};

use crate::{AstrError, Result};

/// 安全获取 StdMutex 锁，中毒时返回 `AstrError::LockPoisoned`。
pub fn lock_anyhow<'a, T>(
    mutex: &'a StdMutex<T>,
    name: &'static str,
) -> Result<StdMutexGuard<'a, T>> {
    mutex
        .lock()
        .map_err(|_| AstrError::LockPoisoned(name.to_string()))
}

/// 安全获取 StdMutex 锁并执行闭包。
///
/// 锁中毒时自动恢复内部状态（`into_inner` + `clear_poison`），
/// 记录 error 日志后继续执行闭包。适用于不可中断的更新操作。
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

/// 安全获取 StdRwLock 读锁，中毒时返回 `AstrError::LockPoisoned`。
pub fn read_lock_anyhow<'a, T>(
    rwlock: &'a StdRwLock<T>,
    name: &'static str,
) -> Result<StdRwLockReadGuard<'a, T>> {
    rwlock
        .read()
        .map_err(|_| AstrError::LockPoisoned(name.to_string()))
}

/// 安全获取 StdRwLock 写锁，中毒时返回 `AstrError::LockPoisoned`。
pub fn write_lock_anyhow<'a, T>(
    rwlock: &'a StdRwLock<T>,
    name: &'static str,
) -> Result<StdRwLockWriteGuard<'a, T>> {
    rwlock
        .write()
        .map_err(|_| AstrError::LockPoisoned(name.to_string()))
}

/// 安全获取 StdRwLock 写锁并执行闭包（中毒时恢复）。
pub fn with_write_lock_recovery<T, R>(
    rwlock: &StdRwLock<T>,
    name: &'static str,
    update: impl FnOnce(&mut T) -> R,
) -> R {
    match rwlock.write() {
        Ok(mut guard) => update(&mut guard),
        Err(poisoned) => {
            log::error!("rwlock '{name}' was poisoned; recovering inner state");
            let mut guard = poisoned.into_inner();
            let result = update(&mut guard);
            rwlock.clear_poison();
            result
        },
    }
}

/// 安全获取 StdRwLock 读锁并执行闭包（中毒时恢复）。
pub fn with_read_lock_recovery<T, R>(
    rwlock: &StdRwLock<T>,
    name: &'static str,
    read: impl FnOnce(&T) -> R,
) -> R {
    match rwlock.read() {
        Ok(guard) => read(&guard),
        Err(poisoned) => {
            log::error!("rwlock '{name}' was poisoned; recovering inner state for read");
            let guard = poisoned.into_inner();
            let result = read(&guard);
            rwlock.clear_poison();
            result
        },
    }
}
