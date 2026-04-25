//! # 应用层治理模型
//!
//! 不依赖具体 runtime service。
//!
//! ## 设计要点
//!
//! - 依赖运行时治理端口和会话信息提供者
//! - 通过运行时治理端口获取运行时标识、插件快照、能力列表和关闭能力
//! - 通过 `SessionInfoProvider` 获取会话计数和活跃会话列表
//! - 可观测性指标通过 `ObservabilitySnapshotProvider` trait 获取
//! - 重载逻辑通过 `RuntimeReloader` trait 委托

use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};

use astrcode_core::CapabilitySpec;
use astrcode_plugin_host::PluginEntry;

use super::TaskRegistry;
use crate::{
    ServerApplicationError,
    observability::{GovernanceSnapshot, ReloadResult, RuntimeObservabilitySnapshot},
};

/// 可观测性指标快照提供者。
///
/// 将指标收集与治理模型解耦。
pub trait ObservabilitySnapshotProvider: Send + Sync {
    fn snapshot(&self) -> RuntimeObservabilitySnapshot;
}

/// 会话信息提供者。
///
/// 抽象会话计数和列表查询。
pub trait SessionInfoProvider: Send + Sync {
    /// 已加载的会话数量。
    fn loaded_session_count(&self) -> usize;

    /// 正在执行中的会话 ID 列表。
    fn running_session_ids(&self) -> Vec<String>;
}

/// 运行时重载策略。
///
/// 封装插件发现、能力面组装和原子替换的完整重载流程。
/// 实际实现由组合根提供。
pub trait RuntimeReloader: Send + Sync {
    /// 执行重载，返回搜索路径列表。
    fn reload(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PathBuf>, ServerApplicationError>> + Send + '_>>;
}

/// 运行时治理快照。
///
/// 这是 `AppGovernance` 需要的最小治理视图，
/// 用于避免应用层直接依赖具体的 `RuntimeCoordinator` 实现。
#[derive(Debug, Clone)]
pub struct RuntimeGovernanceSnapshot {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub capabilities: Vec<CapabilitySpec>,
    pub plugins: Vec<PluginEntry>,
}

/// 运行时治理端口。
///
/// 组合根可以用 `RuntimeCoordinator` 适配它，也可以替换成其他治理实现，
/// 但应用层只依赖这份最小契约。
pub trait RuntimeGovernancePort: Send + Sync {
    fn snapshot(&self) -> RuntimeGovernanceSnapshot;
    fn shutdown(
        &self,
        timeout_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<(), ServerApplicationError>> + Send + '_>>;
}

/// 应用层治理。
///
/// 管理运行时的生命周期、可观测性和重载能力。
/// 通过组合运行时治理端口和 trait 提供者实现所有治理功能。
pub struct AppGovernance {
    runtime: Arc<dyn RuntimeGovernancePort>,
    task_registry: Arc<TaskRegistry>,
    observability: Arc<dyn ObservabilitySnapshotProvider>,
    sessions: Arc<dyn SessionInfoProvider>,
    reloader: Option<Arc<dyn RuntimeReloader>>,
}

impl AppGovernance {
    pub fn new(
        runtime: Arc<dyn RuntimeGovernancePort>,
        task_registry: Arc<TaskRegistry>,
        observability: Arc<dyn ObservabilitySnapshotProvider>,
        sessions: Arc<dyn SessionInfoProvider>,
    ) -> Self {
        Self {
            runtime,
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
        let runtime = self.runtime.snapshot();

        GovernanceSnapshot {
            runtime_name: runtime.runtime_name,
            runtime_kind: runtime.runtime_kind,
            loaded_session_count: self.sessions.loaded_session_count(),
            running_session_ids: self.sessions.running_session_ids(),
            plugin_search_paths,
            metrics: self.observability.snapshot(),
            capabilities: runtime.capabilities,
            plugins: runtime.plugins,
        }
    }

    /// 重载运行时能力面。
    ///
    /// 需要在构造时通过 `with_reloader` 设置重载策略，
    /// 否则返回 `ServerApplicationError::Internal`。
    pub async fn reload(&self) -> Result<ReloadResult, ServerApplicationError> {
        let reloader = self.reloader.as_ref().ok_or_else(|| {
            ServerApplicationError::Internal("no reloader configured".to_string())
        })?;
        let running_sessions = self.sessions.running_session_ids();
        if !running_sessions.is_empty() {
            return Err(ServerApplicationError::Conflict(format!(
                "cannot reload while sessions are running: {}",
                running_sessions.join(", ")
            )));
        }

        let search_paths = reloader.reload().await?;

        Ok(ReloadResult {
            snapshot: self.snapshot(search_paths),
            reloaded_at: chrono::Utc::now(),
        })
    }

    /// 优雅关闭：先停止运行时，再中止所有任务，最后关闭托管组件。
    pub async fn shutdown(&self, timeout_secs: u64) -> Result<(), ServerApplicationError> {
        // 先中止所有后台任务
        let turn_handles = self.task_registry.take_all_turn_handles();
        let subagent_handles = self.task_registry.take_all_subagent_handles();
        for handle in turn_handles.iter().chain(subagent_handles.iter()) {
            handle.abort();
        }

        // 然后关闭运行时和托管组件
        self.runtime.shutdown(timeout_secs).await
    }

    pub fn runtime(&self) -> &Arc<dyn RuntimeGovernancePort> {
        &self.runtime
    }
}

impl std::fmt::Debug for AppGovernance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let runtime = self.runtime.snapshot();
        f.debug_struct("AppGovernance")
            .field("runtime_name", &runtime.runtime_name)
            .finish_non_exhaustive()
    }
}
