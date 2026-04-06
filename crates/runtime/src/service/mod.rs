//! # 运行时服务
//!
//! RuntimeService 是 Astrcode 的核心服务，负责管理会话和执行 Agent 循环。

use std::sync::{
    Arc, RwLock as StdRwLock,
    atomic::{AtomicBool, Ordering},
};

use astrcode_core::{
    AllowAllPolicyEngine, AstrError, HookHandler, PolicyEngine, RuntimeHandle, SessionManager,
};
use astrcode_runtime_agent_control::AgentControl;
use astrcode_runtime_agent_loader::{AgentProfileLoader, AgentProfileRegistry};
use astrcode_runtime_agent_loop::{AgentLoop, ApprovalBroker, DefaultApprovalBroker};
use astrcode_runtime_prompt::PromptDeclaration;
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_session::SessionState;
use astrcode_runtime_skill_loader::SkillCatalog;
use astrcode_storage::session::FileSystemSessionRepository;
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::config::load_config;

#[cfg(test)]
mod baselines;
mod blocking_bridge;
mod composer_ops;
mod config_ops;
mod execution;
mod loop_factory;
mod observability;
mod replay;
mod service_contract;
mod session;
mod turn;
mod watch_ops;

pub(crate) use execution::DeferredSubAgentExecutor;
pub use execution::{
    AgentExecutionServiceHandle, AgentProfileSummary, ToolExecutionServiceHandle, ToolSummary,
};
use observability::RuntimeObservability;
pub use observability::{
    OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};

use self::loop_factory::build_agent_loop;
pub use self::service_contract::{
    AgentExecutionAccepted, ComposerOption, ComposerOptionKind, ComposerOptionsRequest,
    PromptAccepted, ServiceError, ServiceResult, SessionCatalogEvent, SessionEventRecord,
    SessionHistorySnapshot, SessionMessage, SessionReplay, SessionReplaySource,
    SubRunStatusSnapshot,
};

const SESSION_CATALOG_BROADCAST_CAPACITY: usize = 256;

#[derive(Clone)]
struct RuntimeSurfaceState {
    capabilities: CapabilityRouter,
    prompt_declarations: Vec<PromptDeclaration>,
    skill_catalog: Arc<SkillCatalog>,
    hook_handlers: Vec<Arc<dyn HookHandler>>,
}

pub(crate) struct RuntimeServiceDeps {
    agent_loader: Arc<AgentProfileLoader>,
    agent_profiles: Arc<StdRwLock<Arc<AgentProfileRegistry>>>,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalBroker>,
    session_manager: Arc<dyn SessionManager>,
}

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
    /// 当前 runtime surface 的缓存副本。
    ///
    /// 配置热重载只应更新 loop 的配置参数，而不能丢掉插件组装后的 capability
    /// surface，因此这里保留一份可复用的输入快照。
    surface: RwLock<RuntimeSurfaceState>,
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
    /// 子 Agent 控制平面。
    ///
    /// runtime 必须持有这份状态，才能把 parent turn 的取消/结束传播到
    /// 真正的子 Agent 树，而不是停留在独立库测试里。
    agent_control: AgentControl,
    /// Agent 定义加载器。
    ///
    /// loader 负责把 builtin / user / project 三层来源收敛成运行时可见的 profile 快照。
    agent_loader: Arc<AgentProfileLoader>,
    /// Agent Profile 注册表。
    ///
    /// profile 属于 runtime bootstrap 装配结果，不应该由调用方临时拼接；
    /// service 持有同一份只读快照，确保后续子 Agent/工具看到一致配置。
    agent_profiles: Arc<StdRwLock<Arc<AgentProfileRegistry>>>,
    /// 跨窗口共享的会话目录广播。
    /// 新建/删除/分叉会话后会发事件，驱动所有前端窗口刷新 sidebar 或跟随新分支。
    session_catalog_events: broadcast::Sender<SessionCatalogEvent>,
    /// 关闭令牌（用于通知所有处理器停止）
    shutdown_token: CancellationToken,
    /// 序列化 capability reload 与 config reload，避免交错替换 loop。
    rebuild_lock: Mutex<()>,
    /// 防止重复启动配置 watcher。
    config_watch_started: AtomicBool,
    /// 防止重复启动 agent watcher。
    agent_watch_started: AtomicBool,
}

impl RuntimeService {
    pub fn from_capabilities(capabilities: CapabilityRouter) -> ServiceResult<Self> {
        Self::from_capabilities_with_prompt_inputs(
            capabilities,
            Vec::new(),
            Arc::new(SkillCatalog::new(
                astrcode_runtime_skill_loader::load_builtin_skills(),
            )),
        )
    }

    pub fn from_capabilities_with_prompt_inputs(
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
    ) -> ServiceResult<Self> {
        let agent_loader = Arc::new(AgentProfileLoader::new().map_err(ServiceError::from)?);
        Self::from_capabilities_with_prompt_inputs_and_agents(
            capabilities,
            prompt_declarations,
            skill_catalog,
            agent_loader,
            Arc::new(StdRwLock::new(Arc::new(
                AgentProfileRegistry::with_builtin_defaults(),
            ))),
        )
    }

    pub fn from_capabilities_with_prompt_inputs_and_agents(
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
        agent_loader: Arc<AgentProfileLoader>,
        agent_profiles: Arc<StdRwLock<Arc<AgentProfileRegistry>>>,
    ) -> ServiceResult<Self> {
        Self::from_runtime_services(
            capabilities,
            prompt_declarations,
            skill_catalog,
            RuntimeServiceDeps {
                agent_loader,
                agent_profiles,
                policy: Arc::new(AllowAllPolicyEngine),
                approval: Arc::new(DefaultApprovalBroker),
                session_manager: Arc::new(FileSystemSessionRepository),
            },
        )
    }

    pub(crate) fn from_runtime_services(
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
        deps: RuntimeServiceDeps,
    ) -> ServiceResult<Self> {
        let RuntimeServiceDeps {
            agent_loader,
            agent_profiles,
            policy,
            approval,
            session_manager,
        } = deps;
        let config = load_config().map_err(ServiceError::from)?;
        let surface = RuntimeSurfaceState {
            capabilities,
            prompt_declarations,
            skill_catalog,
            hook_handlers: Vec::new(),
        };
        let loop_ = build_agent_loop(
            &surface,
            &config.runtime,
            Arc::clone(&policy),
            Arc::clone(&approval),
        );
        let agent_control = AgentControl::from_config(&config.runtime);
        let (session_catalog_events, _) = broadcast::channel(SESSION_CATALOG_BROADCAST_CAPACITY);
        Ok(Self {
            sessions: DashMap::new(),
            loop_: RwLock::new(loop_),
            surface: RwLock::new(surface),
            policy,
            approval,
            config: Mutex::new(config),
            session_manager,
            session_load_lock: Mutex::new(()),
            observability: Arc::new(RuntimeObservability::default()),
            agent_control,
            agent_loader,
            agent_profiles,
            session_catalog_events,
            shutdown_token: CancellationToken::new(),
            rebuild_lock: Mutex::new(()),
            config_watch_started: AtomicBool::new(false),
            agent_watch_started: AtomicBool::new(false),
        })
    }

    pub async fn current_loop(&self) -> Arc<AgentLoop> {
        self.loop_.read().await.clone()
    }

    #[deprecated(
        note = "会清空 hook_handlers，导致插件 hook 在热替换时静默丢失。请使用 \
                `replace_capabilities_with_prompt_inputs_and_hooks`。"
    )]
    pub async fn replace_capabilities_with_prompt_inputs(
        &self,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
    ) -> ServiceResult<()> {
        self.replace_capabilities_with_prompt_inputs_and_hooks(
            capabilities,
            prompt_declarations,
            skill_catalog,
            Vec::new(),
        )
        .await
    }

    pub async fn replace_capabilities_with_prompt_inputs_and_hooks(
        &self,
        capabilities: CapabilityRouter,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
        hook_handlers: Vec<Arc<dyn HookHandler>>,
    ) -> ServiceResult<()> {
        let _guard = self.rebuild_lock.lock().await;
        let runtime_config = {
            let config = self.config.lock().await;
            config.runtime.clone()
        };
        let next_surface = RuntimeSurfaceState {
            capabilities,
            prompt_declarations,
            skill_catalog,
            hook_handlers,
        };
        let next_loop = build_agent_loop(
            &next_surface,
            &runtime_config,
            Arc::clone(&self.policy),
            Arc::clone(&self.approval),
        );
        *self.loop_.write().await = next_loop;
        *self.surface.write().await = next_surface;
        Ok(())
    }

    pub fn start_config_auto_reload(self: &Arc<Self>) {
        if self
            .config_watch_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let service = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = watch_ops::run_config_watch_loop(service).await {
                log::warn!("config hot reload watcher stopped: {}", error);
            }
        });
    }

    pub fn start_agent_auto_reload(self: &Arc<Self>) {
        if self
            .agent_watch_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let service = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = watch_ops::run_agent_watch_loop(service).await {
                log::warn!("agent hot reload watcher stopped: {}", error);
            }
        });
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

    pub fn agent_control(&self) -> AgentControl {
        self.agent_control.clone()
    }

    pub fn agent_loader(&self) -> Arc<AgentProfileLoader> {
        Arc::clone(&self.agent_loader)
    }

    pub fn agent_profiles(&self) -> Arc<AgentProfileRegistry> {
        self.agent_profiles
            .read()
            .expect("agent profile registry lock should not be poisoned")
            .clone()
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
                let active_turn_id = session
                    .active_turn_id
                    .lock()
                    .ok()
                    .and_then(|guard| guard.clone());
                if let Some(active_turn_id) = active_turn_id {
                    let _ = self
                        .agent_control
                        .cancel_for_parent_turn(&active_turn_id)
                        .await;
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
