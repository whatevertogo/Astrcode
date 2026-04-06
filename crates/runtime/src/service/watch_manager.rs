use std::sync::{Arc, atomic::Ordering};

use super::{RuntimeService, watch_ops};

/// watcher 管理器：封装配置与 agent 自动重载监听器的启动职责。
///
/// 启动幂等与后台任务生命周期都放在这里，避免 RuntimeService 门面继续膨胀。
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
        tokio::spawn(async move {
            if let Err(error) = watch_ops::run_config_watch_loop(service).await {
                log::warn!("config hot reload watcher stopped: {}", error);
            }
        });
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
        tokio::spawn(async move {
            if let Err(error) = watch_ops::run_agent_watch_loop(service).await {
                log::warn!("agent hot reload watcher stopped: {}", error);
            }
        });
    }
}
