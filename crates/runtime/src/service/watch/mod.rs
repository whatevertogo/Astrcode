use std::sync::{Arc, Mutex as StdMutex, atomic::Ordering};

use astrcode_core::support::with_lock_recovery;

use crate::service::RuntimeService;

mod ops;

/// watch 子边界的内部状态。
///
/// 将 watch 相关的 AtomicBool 和 JoinHandle 封装在此，
/// 避免 RuntimeService 门面上直接暴露这些实现细节。
pub(crate) struct WatchState {
    /// 防止重复启动配置 watcher。
    config_watch_started: std::sync::atomic::AtomicBool,
    /// 防止重复启动 agent watcher。
    agent_watch_started: std::sync::atomic::AtomicBool,
    /// 配置热重载 watcher 的 JoinHandle，shutdown 时 abort。
    config_watch_handle: StdMutex<Option<tokio::task::JoinHandle<()>>>,
    /// Agent 定义热重载 watcher 的 JoinHandle，shutdown 时 abort。
    agent_watch_handle: StdMutex<Option<tokio::task::JoinHandle<()>>>,
}

impl WatchState {
    pub(crate) fn new() -> Self {
        Self {
            config_watch_started: std::sync::atomic::AtomicBool::new(false),
            agent_watch_started: std::sync::atomic::AtomicBool::new(false),
            config_watch_handle: StdMutex::new(None),
            agent_watch_handle: StdMutex::new(None),
        }
    }

    /// 尝试标记配置 watcher 为已启动，返回是否成功。
    /// 如果已经启动过，返回 false。
    pub(crate) fn try_start_config_watch(&self) -> bool {
        self.config_watch_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// 尝试标记 agent watcher 为已启动，返回是否成功。
    /// 如果已经启动过，返回 false。
    pub(crate) fn try_start_agent_watch(&self) -> bool {
        self.agent_watch_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// 安装配置 watcher 任务句柄。
    pub(crate) fn install_config_watch_task(&self, handle: tokio::task::JoinHandle<()>) {
        with_lock_recovery(
            &self.config_watch_handle,
            "WatchState.config_watch_handle",
            |guard| {
                *guard = Some(handle);
            },
        );
    }

    /// 安装 agent watcher 任务句柄。
    pub(crate) fn install_agent_watch_task(&self, handle: tokio::task::JoinHandle<()>) {
        with_lock_recovery(
            &self.agent_watch_handle,
            "WatchState.agent_watch_handle",
            |guard| {
                *guard = Some(handle);
            },
        );
    }

    /// 中止所有 watcher 任务并清理句柄。
    pub(crate) fn abort_watchers(&self) {
        if let Some(handle) = self
            .config_watch_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
        if let Some(handle) = self
            .agent_watch_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
    }
}

/// `runtime-watch` 的唯一 surface handle。
#[derive(Clone)]
pub struct WatchServiceHandle {
    runtime: Arc<RuntimeService>,
}

impl WatchServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    pub fn start_config_auto_reload(&self) {
        if !self.runtime.watch_state.try_start_config_watch() {
            return;
        }

        let service = Arc::clone(&self.runtime);
        let handle = tokio::spawn(async move {
            if let Err(error) = ops::run_config_watch_loop(service).await {
                log::warn!("config hot reload watcher stopped: {}", error);
            }
        });
        self.runtime.lifecycle().install_config_watch_task(handle);
    }

    pub fn start_agent_auto_reload(&self) {
        if !self.runtime.watch_state.try_start_agent_watch() {
            return;
        }

        let service = Arc::clone(&self.runtime);
        let handle = tokio::spawn(async move {
            if let Err(error) = ops::run_agent_watch_loop(service).await {
                log::warn!("agent hot reload watcher stopped: {}", error);
            }
        });
        self.runtime.lifecycle().install_agent_watch_task(handle);
    }
}
