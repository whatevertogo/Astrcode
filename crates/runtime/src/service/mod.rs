//! # 运行时服务
//!
//! RuntimeService 是 Astrcode 的核心服务，负责管理会话和执行 Agent 循环。

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::agent_loop::AgentLoop;
use crate::approval_service::{ApprovalBroker, DefaultApprovalBroker};
use crate::config::{
    load_config, resolve_auto_compact_enabled, resolve_compact_keep_recent_turns,
    resolve_compact_threshold_percent, resolve_max_tool_concurrency, resolve_tool_result_max_bytes,
};
use crate::prompt::{PromptDeclaration, SkillSpec};
use crate::provider_factory::ConfigFileProviderFactory;
use astrcode_core::{
    AllowAllPolicyEngine, AstrError, CapabilityRouter, PolicyEngine, RuntimeHandle, SessionManager,
};
use astrcode_storage::session::FileSystemSessionRepository;

#[cfg(test)]
mod baselines;
mod config_ops;
mod observability;
mod replay;
mod session_ops;
mod session_state;
mod support;
mod turn_ops;
mod types;

use self::session_state::SessionState;
pub use self::types::{
    PromptAccepted, ServiceError, ServiceResult, SessionCatalogEvent, SessionEventRecord,
    SessionMessage, SessionReplay, SessionReplaySource,
};
use observability::RuntimeObservability;
pub use observability::{
    OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
};

const SESSION_CATALOG_BROADCAST_CAPACITY: usize = 256;

/// 运行时服务
///
/// 负责管理所有会话的状态和执行。主要职责：
/// - 会话生命周期管理（创建、加载、删除）
/// - Agent Turn 执行（通过 AgentLoop）
/// - 事件流广播（SSE）
/// - 优雅关闭
pub struct RuntimeService {
    /// 会话 ID -> 会话状态的映射（使用 DashMap 支持并发访问）
    sessions: DashMap<String, Arc<SessionState>>,
    /// Agent Loop 实例（可热替换，用于支持运行时重载能力）
    loop_: RwLock<Arc<AgentLoop>>,
    /// 策略引擎（控制能力调用是否需要审批）
    policy: Arc<dyn PolicyEngine>,
    /// 审批代理（处理用户确认流程）
    approval: Arc<dyn ApprovalBroker>,
    /// 配置（API 密钥等）
    config: Mutex<crate::config::Config>,
    /// 会话存储实现（负责 durable session 文件读写）
    session_manager: Arc<dyn SessionManager>,
    /// 会话加载锁（防止并发加载同一会话）。
    /// `Mutex<()>` 是 Tokio 中常见的"旋转门"模式——guard 不持有任何数据，
    /// 仅用于确保互斥。相比使用专门的 AtomicBool + Notify 机制，
    /// 这种方式更简洁且编译器能更好地优化。
    session_load_lock: Mutex<()>,
    /// 可观测性（指标收集）
    observability: Arc<RuntimeObservability>,
    /// 跨窗口共享的会话目录广播。
    /// 新建/删除/分叉会话后会发事件，驱动所有前端窗口刷新 sidebar 或跟随新分支。
    session_catalog_events: broadcast::Sender<SessionCatalogEvent>,
    /// 关闭令牌（用于通知所有处理器停止）
    shutdown_token: CancellationToken,
}

impl RuntimeService {
    pub fn from_capabilities(capabilities: CapabilityRouter) -> ServiceResult<Self> {
        Self::from_capabilities_with_prompt_inputs(
            capabilities,
            Vec::new(),
            crate::builtin_skills::builtin_skills(),
        )
    }

    pub fn from_capabilities_with_prompt_inputs(
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        prompt_skills: Vec<SkillSpec>,
    ) -> ServiceResult<Self> {
        Self::from_runtime_services(
            capabilities,
            prompt_declarations,
            prompt_skills,
            Arc::new(AllowAllPolicyEngine),
            Arc::new(DefaultApprovalBroker),
            Arc::new(FileSystemSessionRepository),
        )
    }

    pub fn from_runtime_services(
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        prompt_skills: Vec<SkillSpec>,
        policy: Arc<dyn PolicyEngine>,
        approval: Arc<dyn ApprovalBroker>,
        session_manager: Arc<dyn SessionManager>,
    ) -> ServiceResult<Self> {
        let config = load_config().map_err(ServiceError::from)?;
        let max_tool_concurrency = resolve_max_tool_concurrency(&config.runtime);
        let loop_ = AgentLoop::from_capabilities_with_prompt_inputs(
            Arc::new(ConfigFileProviderFactory),
            capabilities,
            prompt_declarations,
            prompt_skills,
        )
        .with_max_tool_concurrency(max_tool_concurrency)
        .with_auto_compact_enabled(resolve_auto_compact_enabled(&config.runtime))
        .with_compact_threshold_percent(resolve_compact_threshold_percent(&config.runtime))
        .with_tool_result_max_bytes(resolve_tool_result_max_bytes(&config.runtime))
        .with_compact_keep_recent_turns(resolve_compact_keep_recent_turns(&config.runtime) as usize)
        .with_policy_engine(Arc::clone(&policy))
        .with_approval_broker(Arc::clone(&approval));
        let (session_catalog_events, _) = broadcast::channel(SESSION_CATALOG_BROADCAST_CAPACITY);
        Ok(Self {
            sessions: DashMap::new(),
            loop_: RwLock::new(Arc::new(loop_)),
            policy,
            approval,
            config: Mutex::new(config),
            session_manager,
            session_load_lock: Mutex::new(()),
            observability: Arc::new(RuntimeObservability::default()),
            session_catalog_events,
            shutdown_token: CancellationToken::new(),
        })
    }

    pub async fn current_loop(&self) -> Arc<AgentLoop> {
        self.loop_.read().await.clone()
    }

    pub async fn replace_capabilities_with_prompt_inputs(
        &self,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        prompt_skills: Vec<SkillSpec>,
    ) -> ServiceResult<()> {
        let runtime_config = {
            let config = self.config.lock().await;
            config.runtime.clone()
        };
        let max_tool_concurrency = resolve_max_tool_concurrency(&runtime_config);
        let next_loop = Arc::new(
            AgentLoop::from_capabilities_with_prompt_inputs(
                Arc::new(ConfigFileProviderFactory),
                capabilities,
                prompt_declarations,
                prompt_skills,
            )
            .with_max_tool_concurrency(max_tool_concurrency)
            .with_auto_compact_enabled(resolve_auto_compact_enabled(&runtime_config))
            .with_compact_threshold_percent(resolve_compact_threshold_percent(&runtime_config))
            .with_tool_result_max_bytes(resolve_tool_result_max_bytes(&runtime_config))
            .with_compact_keep_recent_turns(
                resolve_compact_keep_recent_turns(&runtime_config) as usize
            )
            .with_policy_engine(Arc::clone(&self.policy))
            .with_approval_broker(Arc::clone(&self.approval)),
        );
        // 写锁会阻塞直到所有活跃 reader（即正在运行的 turn 通过 current_loop()
        // 持有的读锁）释放。已运行的 turn 继续使用旧的 AgentLoop（通过 Arc 引用），
        // 新 turn 则获取新的 loop。这是一种优雅的滚动替换模式——无需暂停服务。
        *self.loop_.write().await = next_loop;
        Ok(())
    }

    pub fn loaded_session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn running_session_ids(&self) -> Vec<String> {
        let mut running_sessions = self
            .sessions
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .running
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        running_sessions.sort();
        running_sessions
    }

    pub fn observability_snapshot(&self) -> RuntimeObservabilitySnapshot {
        self.observability.snapshot()
    }

    pub fn subscribe_session_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.session_catalog_events.subscribe()
    }

    pub(super) fn emit_session_catalog_event(&self, event: SessionCatalogEvent) {
        let _ = self.session_catalog_events.send(event);
    }

    /// Initiates graceful shutdown:
    /// 1. Signals all running turns to cancel
    /// 2. Waits for all turns to complete (with timeout)
    /// 3. Returns when all sessions are idle or timeout elapsed
    pub async fn shutdown(&self, timeout_secs: u64) {
        log::info!("Initiating graceful shutdown...");

        // Signal shutdown to all handlers
        self.shutdown_token.cancel();

        // Cancel all running sessions
        for entry in self.sessions.iter() {
            let session = entry.value();
            if session.running.load(std::sync::atomic::Ordering::SeqCst) {
                if let Ok(cancel) = session.cancel.lock().map(|g| g.clone()) {
                    cancel.cancel();
                }
            }
        }

        // Wait for all sessions to become idle
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            let running_count = self
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

            // 100ms 轮询检查所有会话是否空闲。使用轮询而非 push-based 通知
            // （如 watch channel / Notify）是因为 shutdown 是低频操作，添加通知
            // 机制需要在每个 turn 完成路径中增加额外的唤醒逻辑，收益不大。
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

#[async_trait]
impl RuntimeHandle for RuntimeService {
    fn runtime_name(&self) -> &'static str {
        "astrcode-runtime"
    }

    fn runtime_kind(&self) -> &'static str {
        "native"
    }

    async fn shutdown(&self, timeout_secs: u64) -> std::result::Result<(), AstrError> {
        RuntimeService::shutdown(self, timeout_secs).await;
        Ok(())
    }
}
