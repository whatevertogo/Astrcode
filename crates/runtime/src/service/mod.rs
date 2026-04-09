//! # 运行时服务
//!
//! RuntimeService 是 Astrcode 的核心服务，负责管理会话和执行 Agent 循环。

use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock, atomic::AtomicBool},
};

use astrcode_core::{
    AgentProfile, AgentProfileCatalog, AllowAllPolicyEngine, AstrError, HookHandler, PolicyEngine,
    RuntimeHandle, SessionManager,
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
mod capability_manager;
mod composer_ops;
mod config_manager;
mod config_ops;
mod execution;
mod loop_factory;
mod observability;
mod service_contract;
mod session;
mod turn;
mod watch_manager;
mod watch_ops;

pub use execution::{
    AgentExecutionServiceHandle, AgentProfileSummary, ToolExecutionServiceHandle, ToolSummary,
};
pub(crate) use execution::{DeferredCollaborationExecutor, DeferredSubAgentExecutor};
use observability::RuntimeObservability;
pub use observability::{
    OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
pub use session::SessionServiceHandle;

use self::loop_factory::{LoopRuntimeDeps, build_agent_loop};
pub use self::service_contract::{
    AgentExecutionAccepted, ComposerOption, ComposerOptionKind, ComposerOptionsRequest,
    PromptAccepted, ServiceError, ServiceResult, SessionCatalogEvent, SessionEventRecord,
    SessionHistorySnapshot, SessionReplay, SessionReplaySource, SubRunStatusSnapshot,
    SubRunStatusSource,
};

const SESSION_CATALOG_BROADCAST_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwnerGraph {
    pub session_owner: &'static str,
    pub execution_owner: &'static str,
    pub tool_owner: &'static str,
}

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

#[derive(Clone)]
struct RuntimeAgentProfileCatalog {
    agent_profiles: Arc<StdRwLock<Arc<AgentProfileRegistry>>>,
}

impl RuntimeAgentProfileCatalog {
    fn new(agent_profiles: Arc<StdRwLock<Arc<AgentProfileRegistry>>>) -> Self {
        Self { agent_profiles }
    }
}

impl AgentProfileCatalog for RuntimeAgentProfileCatalog {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile> {
        self.agent_profiles
            .read()
            .expect("agent profile registry lock should not be poisoned")
            .list_subagent_profiles()
            .into_iter()
            .cloned()
            .collect()
    }
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
    /// scoped agent profile 缓存。
    ///
    /// `spawnAgent` 会反复查询同一个 working dir 的 profile 视图；如果每次都重新扫盘，
    /// 子 agent 冷启动前的同步 IO 会明显拖慢工具返回。这里缓存“按目录解析后的注册表”，
    /// 并在 agent 热重载后统一失效，保持语义集中在 runtime 层。
    scoped_agent_profiles: DashMap<PathBuf, Arc<AgentProfileRegistry>>,
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
    /// 配置热重载 watcher 的 JoinHandle，shutdown 时 abort。
    config_watch_handle: StdMutex<Option<tokio::task::JoinHandle<()>>>,
    /// Agent 定义热重载 watcher 的 JoinHandle，shutdown 时 abort。
    agent_watch_handle: StdMutex<Option<tokio::task::JoinHandle<()>>>,
    /// 活跃的子 Agent 后台执行任务的 JoinHandle，shutdown 时批量 abort。
    active_subagent_handles: StdMutex<Vec<tokio::task::JoinHandle<()>>>,
    /// 活跃的 turn 执行任务的 JoinHandle，shutdown 时批量 abort。
    active_turn_handles: StdMutex<Vec<tokio::task::JoinHandle<()>>>,
    /// 协作工具的延迟执行器桥（runtime surface 热重载时复用）。
    collaboration_executor: Arc<DeferredCollaborationExecutor>,
}

impl RuntimeService {
    pub fn owner_graph(&self) -> RuntimeOwnerGraph {
        RuntimeOwnerGraph {
            session_owner: "runtime-session",
            execution_owner: "runtime-execution",
            tool_owner: "runtime-execution",
        }
    }

    fn capability_manager(&self) -> capability_manager::CapabilityManager<'_> {
        capability_manager::CapabilityManager::new(self)
    }

    fn config_manager(&self) -> config_manager::ConfigManager<'_> {
        config_manager::ConfigManager::new(self)
    }

    fn watch_manager(self: &Arc<Self>) -> watch_manager::WatchManager {
        watch_manager::WatchManager::new(Arc::clone(self))
    }

    fn agent_profile_catalog(&self) -> Arc<dyn AgentProfileCatalog> {
        Arc::new(RuntimeAgentProfileCatalog::new(Arc::clone(
            &self.agent_profiles,
        )))
    }

    /// 获取协作工具执行器的引用（用于 surface 热重载）。
    pub(crate) fn collaboration_executor(&self) -> Arc<DeferredCollaborationExecutor> {
        Arc::clone(&self.collaboration_executor)
    }

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
            &config.active_profile,
            &config.runtime,
            LoopRuntimeDeps::new(
                Arc::clone(&policy),
                Arc::clone(&approval),
                Some(Arc::new(RuntimeAgentProfileCatalog::new(Arc::clone(
                    &agent_profiles,
                )))),
            ),
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
            scoped_agent_profiles: DashMap::new(),
            session_catalog_events,
            shutdown_token: CancellationToken::new(),
            rebuild_lock: Mutex::new(()),
            config_watch_started: AtomicBool::new(false),
            agent_watch_started: AtomicBool::new(false),
            config_watch_handle: StdMutex::new(None),
            agent_watch_handle: StdMutex::new(None),
            active_subagent_handles: StdMutex::new(Vec::new()),
            active_turn_handles: StdMutex::new(Vec::new()),
            collaboration_executor: Arc::new(DeferredCollaborationExecutor::default()),
        })
    }

    pub async fn current_loop(&self) -> Arc<AgentLoop> {
        self.capability_manager().current_loop().await
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
        self.capability_manager()
            .replace_surface(
                capabilities,
                prompt_declarations,
                skill_catalog,
                hook_handlers,
            )
            .await
    }

    pub fn start_config_auto_reload(self: &Arc<Self>) {
        self.watch_manager().start_config_auto_reload();
    }

    pub fn start_agent_auto_reload(self: &Arc<Self>) {
        self.watch_manager().start_agent_auto_reload();
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

    pub(super) async fn known_agent_working_dirs(&self) -> ServiceResult<Vec<PathBuf>> {
        let session_manager = Arc::clone(&self.session_manager);
        blocking_bridge::spawn_blocking_service("list agent working dirs", move || {
            session_manager
                .list_sessions_with_meta()
                .map(|metas| {
                    let mut working_dirs = metas
                        .into_iter()
                        .map(|meta| PathBuf::from(meta.working_dir))
                        .collect::<Vec<_>>();
                    working_dirs.sort();
                    working_dirs.dedup();
                    working_dirs
                })
                .map_err(ServiceError::from)
        })
        .await
    }

    pub(super) fn emit_session_catalog_event(&self, event: SessionCatalogEvent) {
        // 故意忽略：通道关闭表示服务已关闭，无需处理
        let _ = self.session_catalog_events.send(event);
    }

    /// Initiates graceful shutdown:
    /// 1. Signals all running turns to cancel
    /// 2. Waits for all turns to complete (with timeout)
    /// 3. Returns when all sessions are idle or timeout elapsed
    pub async fn shutdown(&self, timeout_secs: u64) {
        log::info!("Initiating graceful shutdown...");

        // 中止后台 watcher 任务
        watch_manager::WatchManager::shutdown(self);

        // 中止所有活跃的子 Agent 后台执行任务
        let subagent_handles = std::mem::take(
            &mut *self
                .active_subagent_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for handle in subagent_handles {
            handle.abort();
        }

        // 中止所有活跃的 turn 执行任务
        let turn_handles = std::mem::take(
            &mut *self
                .active_turn_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for handle in turn_handles {
            handle.abort();
        }

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
                    .ok() // 故意忽略：mutex 中毒表示关闭期竞争，安全地跳过
                    .and_then(|guard| guard.clone());
                if let Some(active_turn_id) = active_turn_id {
                    // 故意忽略：取消子运行时失败不应阻断关闭流程
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
