//! 生命周期管理：任务注册表、治理模型与 shutdown 协调。
//!
//! 从 `runtime/service/lifecycle/` 迁入核心任务管理逻辑。
//! `TaskRegistry` 是自包含的，不依赖 `RuntimeService`。
//! `AppGovernance` 替代旧 `RuntimeGovernance`，依赖 `App` 而非 `RuntimeService`。

pub mod governance;

use astrcode_core::support::with_lock_recovery;

/// 活跃任务注册表，跟踪 turn 和 subagent 的 JoinHandle。
///
/// 每次注册新任务时会清理已完成的旧 handle，防止内存无限增长。
/// shutdown 时 `take_all_*` 批量 abort 所有剩余任务。
pub struct TaskRegistry {
    /// 活跃的子 Agent 后台执行任务的 JoinHandle。
    subagent_handles: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// 活跃的 turn 执行任务的 JoinHandle。
    turn_handles: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

/// 清理 Vec 中已完成的 JoinHandle，释放其持有的 Arc<Task> 内存。
fn prune_completed_handles(handles: &mut Vec<tokio::task::JoinHandle<()>>) {
    handles.retain(|h| !h.is_finished());
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            subagent_handles: std::sync::Mutex::new(Vec::new()),
            turn_handles: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 注册 turn 任务句柄，同时清理已完成的旧 handle。
    pub fn register_turn_task(&self, handle: tokio::task::JoinHandle<()>) {
        with_lock_recovery(&self.turn_handles, "TaskRegistry.turn_handles", |guard| {
            prune_completed_handles(guard);
            guard.push(handle);
        });
    }

    /// 注册子 Agent 任务句柄，同时清理已完成的旧 handle。
    pub fn register_subagent_task(&self, handle: tokio::task::JoinHandle<()>) {
        with_lock_recovery(
            &self.subagent_handles,
            "TaskRegistry.subagent_handles",
            |guard| {
                prune_completed_handles(guard);
                guard.push(handle);
            },
        );
    }

    /// 取出所有 turn 任务句柄用于 abort。
    pub fn take_all_turn_handles(&self) -> Vec<tokio::task::JoinHandle<()>> {
        std::mem::take(
            &mut *self
                .turn_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// 取出所有子 Agent 任务句柄用于 abort。
    pub fn take_all_subagent_handles(&self) -> Vec<tokio::task::JoinHandle<()>> {
        std::mem::take(
            &mut *self
                .subagent_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

impl std::fmt::Debug for TaskRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskRegistry").finish_non_exhaustive()
    }
}
