//! # 应用层治理模型
//!
//! 替代旧 `RuntimeGovernance`，不依赖 `RuntimeService`。
//!
//! ## 设计要点
//!
//! - 依赖 `RuntimeCoordinator` 和会话信息提供者，而非旧 `RuntimeService`
//! - 通过 `RuntimeCoordinator` 获取运行时句柄、插件注册表、能力列表
//! - 通过 `SessionInfoProvider` 获取会话计数和活跃会话列表
//! - 可观测性指标通过 `ObservabilitySnapshotProvider` trait 获取， Phase 10 组合根接线时桥接旧
//!   runtime 的实际收集器
//! - 重载逻辑通过 `RuntimeReloader` trait 委托，Phase 10 实现具体组装

use std::{path::PathBuf, sync::Arc};

use astrcode_core::RuntimeCoordinator;

use super::TaskRegistry;
use crate::{
    ApplicationError,
    observability::{GovernanceSnapshot, ReloadResult, RuntimeObservabilitySnapshot},
};

/// 可观测性指标快照提供者。
///
/// 将指标收集与治理模型解耦。实际实现在 Phase 10 组合根中桥接旧 runtime 的
/// `RuntimeObservability` 收集器。
pub trait ObservabilitySnapshotProvider: Send + Sync {
    fn snapshot(&self) -> RuntimeObservabilitySnapshot;
}

/// 会话信息提供者。
///
/// 抽象会话计数和列表查询，过渡期由旧 runtime 实现，
/// 后续由 `SessionRuntime` 直接实现。
pub trait SessionInfoProvider: Send + Sync {
    /// 已加载的会话数量。
    fn loaded_session_count(&self) -> usize;

    /// 正在执行中的会话 ID 列表。
    fn running_session_ids(&self) -> Vec<String>;
}

/// 运行时重载策略。
///
/// 封装插件发现、能力面组装和原子替换的完整重载流程。
/// 实际实现在 Phase 10 组合根中桥接旧 runtime 的 `assemble_runtime_surface`。
pub trait RuntimeReloader: Send + Sync {
    /// 执行重载，返回搜索路径列表。
    fn reload(&self) -> Result<Vec<PathBuf>, ApplicationError>;
}

/// 应用层治理。
///
/// 管理运行时的生命周期、可观测性和重载能力。
/// 不持有旧 `RuntimeService` 引用，通过组合 `RuntimeCoordinator`
/// 和 trait 提供者实现所有治理功能。
pub struct AppGovernance {
    coordinator: Arc<RuntimeCoordinator>,
    task_registry: Arc<TaskRegistry>,
    observability: Arc<dyn ObservabilitySnapshotProvider>,
    sessions: Arc<dyn SessionInfoProvider>,
    reloader: Option<Arc<dyn RuntimeReloader>>,
}

impl AppGovernance {
    pub fn new(
        coordinator: Arc<RuntimeCoordinator>,
        task_registry: Arc<TaskRegistry>,
        observability: Arc<dyn ObservabilitySnapshotProvider>,
        sessions: Arc<dyn SessionInfoProvider>,
    ) -> Self {
        Self {
            coordinator,
            task_registry,
            observability,
            sessions,
            reloader: None,
        }
    }

    /// 设置重载策略。
    pub fn with_reloader(mut self, reloader: Arc<dyn RuntimeReloader>) -> Self {
        self.reloader = Some(reloader);
        self
    }

    /// 获取当前运行时治理快照。
    pub fn snapshot(&self, plugin_search_paths: Vec<PathBuf>) -> GovernanceSnapshot {
        let runtime = self.coordinator.runtime();

        GovernanceSnapshot {
            runtime_name: runtime.runtime_name().to_string(),
            runtime_kind: runtime.runtime_kind().to_string(),
            loaded_session_count: self.sessions.loaded_session_count(),
            running_session_ids: self.sessions.running_session_ids(),
            plugin_search_paths,
            metrics: self.observability.snapshot(),
            capabilities: self.coordinator.capabilities(),
            plugins: self.coordinator.plugin_registry().snapshot(),
        }
    }

    /// 重载运行时能力面。
    ///
    /// 需要在构造时通过 `with_reloader` 设置重载策略，
    /// 否则返回 `ApplicationError::Internal`。
    pub async fn reload(&self) -> Result<ReloadResult, ApplicationError> {
        let reloader = self
            .reloader
            .as_ref()
            .ok_or_else(|| ApplicationError::Internal("no reloader configured".to_string()))?;

        let search_paths = reloader.reload()?;

        Ok(ReloadResult {
            snapshot: self.snapshot(search_paths),
            reloaded_at: chrono::Utc::now(),
        })
    }

    /// 优雅关闭：先停止运行时，再中止所有任务，最后关闭托管组件。
    pub async fn shutdown(&self, timeout_secs: u64) -> Result<(), ApplicationError> {
        // 先中止所有后台任务
        let turn_handles = self.task_registry.take_all_turn_handles();
        let subagent_handles = self.task_registry.take_all_subagent_handles();
        for handle in turn_handles.iter().chain(subagent_handles.iter()) {
            handle.abort();
        }

        // 然后关闭运行时和托管组件
        self.coordinator
            .shutdown(timeout_secs)
            .await
            .map_err(|e| ApplicationError::Internal(e.to_string()))
    }

    pub fn coordinator(&self) -> &Arc<RuntimeCoordinator> {
        &self.coordinator
    }

    pub fn task_registry(&self) -> &Arc<TaskRegistry> {
        &self.task_registry
    }
}

impl std::fmt::Debug for AppGovernance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppGovernance")
            .field("runtime_name", &self.coordinator.runtime().runtime_name())
            .finish_non_exhaustive()
    }
}
