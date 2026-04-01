//! # 运行时服务
//!
//! RuntimeService 是 Astrcode 的核心服务，负责管理会话和执行 Agent 循环。

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::agent_loop::AgentLoop;
use crate::approval_service::{ApprovalBroker, DefaultApprovalBroker};
use crate::config::load_config;
use crate::prompt::{PromptDeclaration, SkillSpec};
use crate::provider_factory::ConfigFileProviderFactory;
use astrcode_core::{
    AllowAllPolicyEngine, AstrError, CapabilityRouter, PolicyEngine, RuntimeHandle,
};

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
    PromptAccepted, ServiceError, ServiceResult, SessionEventRecord, SessionMessage, SessionReplay,
    SessionReplaySource,
};
use observability::RuntimeObservability;
pub use observability::{
    OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
};

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
    /// 会话加载锁（防止并发加载同一会话）
    session_load_lock: Mutex<()>,
    /// 可观测性（指标收集）
    observability: Arc<RuntimeObservability>,
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
        )
    }

    pub fn from_runtime_services(
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        prompt_skills: Vec<SkillSpec>,
        policy: Arc<dyn PolicyEngine>,
        approval: Arc<dyn ApprovalBroker>,
    ) -> ServiceResult<Self> {
        let config = load_config().map_err(ServiceError::from)?;
        let loop_ = AgentLoop::from_capabilities_with_prompt_inputs(
            Arc::new(ConfigFileProviderFactory),
            capabilities,
            prompt_declarations,
            prompt_skills,
        )
        .with_policy_engine(Arc::clone(&policy))
        .with_approval_broker(Arc::clone(&approval));
        Ok(Self {
            sessions: DashMap::new(),
            loop_: RwLock::new(Arc::new(loop_)),
            policy,
            approval,
            config: Mutex::new(config),
            session_load_lock: Mutex::new(()),
            observability: Arc::new(RuntimeObservability::default()),
            shutdown_token: CancellationToken::new(),
        })
    }

    pub async fn current_loop(&self) -> Arc<AgentLoop> {
        self.loop_.read().await.clone()
    }

    pub async fn replace_capabilities(&self, capabilities: CapabilityRouter) -> ServiceResult<()> {
        self.replace_capabilities_with_prompt_inputs(
            capabilities,
            Vec::new(),
            crate::builtin_skills::builtin_skills(),
        )
        .await
    }

    pub async fn replace_capabilities_with_prompt_inputs(
        &self,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        prompt_skills: Vec<SkillSpec>,
    ) -> ServiceResult<()> {
        let next_loop = Arc::new(
            AgentLoop::from_capabilities_with_prompt_inputs(
                Arc::new(ConfigFileProviderFactory),
                capabilities,
                prompt_declarations,
                prompt_skills,
            )
            .with_policy_engine(Arc::clone(&self.policy))
            .with_approval_broker(Arc::clone(&self.approval)),
        );
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

    /// Returns a clone of the shutdown token for use in handlers
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown_token.clone()
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
