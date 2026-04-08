use std::sync::{Arc, atomic::Ordering};

use astrcode_core::support::with_lock_recovery;

use super::{RuntimeService, watch_ops};

/// watcher 管理器：封装配置与 agent 自动重载监听器的启动职责。
///
/// 启动幂等与后台任务生命周期都放在这里，避免 RuntimeService 门面继续膨胀。
/// JoinHandle 保存在 RuntimeService 的字段中，以便 shutdown 时 abort。
pub(super) struct WatchManager {
    runtime: Arc<RuntimeService>,
}

impl WatchManager {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    pub(super) fn start_config_auto_reload(&self) {
        if self
            .runtime
            .config_watch_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let service = Arc::clone(&self.runtime);
        let handle = tokio::spawn(async move {
            if let Err(error) = watch_ops::run_config_watch_loop(service).await {
                log::warn!("config hot reload watcher stopped: {}", error);
            }
        });
        // JoinHandle 保存在 RuntimeService 上，因为 WatchManager 是临时构造的包装器。
        with_lock_recovery(
            &self.runtime.config_watch_handle,
            "RuntimeService.config_watch_handle",
            |guard| {
                *guard = Some(handle);
            },
        );
    }

    pub(super) fn start_agent_auto_reload(&self) {
        if self
            .runtime
            .agent_watch_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let service = Arc::clone(&self.runtime);
        let handle = tokio::spawn(async move {
            if let Err(error) = watch_ops::run_agent_watch_loop(service).await {
                log::warn!("agent hot reload watcher stopped: {}", error);
            }
        });
        with_lock_recovery(
            &self.runtime.agent_watch_handle,
            "RuntimeService.agent_watch_handle",
            |guard| {
                *guard = Some(handle);
            },
        );
    }

    /// 中止配置和 agent 两个后台 watcher 任务。
    pub(super) fn shutdown(runtime: &RuntimeService) {
        if let Some(handle) = runtime
            .config_watch_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
        if let Some(handle) = runtime
            .agent_watch_handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            handle.abort();
        }
    }
}
