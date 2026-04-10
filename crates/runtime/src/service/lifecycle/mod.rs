use std::sync::Arc;

use astrcode_core::support::with_lock_recovery;
use tokio_util::sync::CancellationToken;

use crate::service::RuntimeService;

/// lifecycle 子边界的内部任务注册表。
///
/// 将活跃的 turn/subagent JoinHandle 封装在此，
/// 避免 RuntimeService 门面上直接暴露这些实现细节。
pub(crate) struct TaskRegistry {
    /// 活跃的子 Agent 后台执行任务的 JoinHandle，shutdown 时批量 abort。
    active_subagent_handles: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// 活跃的 turn 执行任务的 JoinHandle，shutdown 时批量 abort。
    active_turn_handles: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl TaskRegistry {
    pub(crate) fn new() -> Self {
        Self {
            active_subagent_handles: std::sync::Mutex::new(Vec::new()),
            active_turn_handles: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 注册 turn 任务句柄。
    pub(crate) fn register_turn_task(&self, handle: tokio::task::JoinHandle<()>) {
        with_lock_recovery(
            &self.active_turn_handles,
            "TaskRegistry.active_turn_handles",
            |guard| guard.push(handle),
        );
    }

    /// 注册子 Agent 任务句柄。
    pub(crate) fn register_subagent_task(&self, handle: tokio::task::JoinHandle<()>) {
        with_lock_recovery(
            &self.active_subagent_handles,
            "TaskRegistry.active_subagent_handles",
            |guard| guard.push(handle),
        );
    }

    /// 取出所有 turn 任务句柄用于 abort。
    pub(crate) fn take_all_turn_handles(&self) -> Vec<tokio::task::JoinHandle<()>> {
        std::mem::take(
            &mut *self
                .active_turn_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// 取出所有子 Agent 任务句柄用于 abort。
    pub(crate) fn take_all_subagent_handles(&self) -> Vec<tokio::task::JoinHandle<()>> {
        std::mem::take(
            &mut *self
                .active_subagent_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

/// `runtime-lifecycle` 的唯一 surface handle。
#[derive(Clone)]
pub struct LifecycleServiceHandle {
    runtime: Arc<RuntimeService>,
}

pub(crate) struct LifecycleService<'a> {
    runtime: &'a RuntimeService,
}

impl<'a> LifecycleService<'a> {
    pub(crate) fn new(runtime: &'a RuntimeService) -> Self {
        Self { runtime }
    }

    pub(crate) fn shutdown_signal(&self) -> CancellationToken {
        self.runtime.shutdown_token.clone()
    }

    pub(crate) fn register_turn_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.runtime.task_registry.register_turn_task(handle);
    }

    pub(crate) fn register_subagent_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.runtime.task_registry.register_subagent_task(handle);
    }

    pub(crate) fn install_config_watch_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.runtime.watch_state.install_config_watch_task(handle);
    }

    pub(crate) fn install_agent_watch_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.runtime.watch_state.install_agent_watch_task(handle);
    }

    pub(crate) async fn shutdown(&self, timeout_secs: u64) {
        log::info!("Initiating graceful shutdown...");

        self.runtime.watch_state.abort_watchers();

        let subagent_handles = self.runtime.task_registry.take_all_subagent_handles();
        for handle in subagent_handles {
            handle.abort();
        }

        let turn_handles = self.runtime.task_registry.take_all_turn_handles();
        for handle in turn_handles {
            handle.abort();
        }

        self.runtime.shutdown_token.cancel();

        for entry in self.runtime.sessions.iter() {
            let session = entry.value();
            if session.running.load(std::sync::atomic::Ordering::SeqCst) {
                if let Ok(cancel) = session.cancel.lock().map(|g| g.clone()) {
                    cancel.cancel();
                }
                let active_turn_id = session
                    .active_turn_id
                    .lock()
                    .ok()
                    .and_then(|guard| guard.clone());
                if let Some(active_turn_id) = active_turn_id {
                    let _ = self
                        .runtime
                        .agent_control
                        .cancel_for_parent_turn(&active_turn_id)
                        .await;
                }
            }
        }

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            let running_count = self
                .runtime
                .sessions
                .iter()
                .filter(|entry| {
                    entry
                        .value()
                        .running
                        .load(std::sync::atomic::Ordering::SeqCst)
                })
                .count();

            if running_count == 0 {
                log::info!("All sessions are idle, shutdown complete");
                return;
            }

            if start.elapsed() >= timeout {
                log::warn!(
                    "Shutdown timeout elapsed, {} sessions still running",
                    running_count
                );
                return;
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

impl LifecycleServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    fn service(&self) -> LifecycleService<'_> {
        LifecycleService::new(self.runtime.as_ref())
    }

    pub(crate) fn shutdown_signal(&self) -> CancellationToken {
        self.service().shutdown_signal()
    }

    pub(crate) fn register_turn_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.service().register_turn_task(handle);
    }

    pub(crate) fn register_subagent_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.service().register_subagent_task(handle);
    }

    pub(crate) fn install_config_watch_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.service().install_config_watch_task(handle);
    }

    pub(crate) fn install_agent_watch_task(&self, handle: tokio::task::JoinHandle<()>) {
        self.service().install_agent_watch_task(handle);
    }

    pub async fn shutdown(&self, timeout_secs: u64) {
        self.service().shutdown(timeout_secs).await;
    }
}
