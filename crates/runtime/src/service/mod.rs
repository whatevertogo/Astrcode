//! # 运行时服务
//!
//! RuntimeService 是 Astrcode 的核心服务，负责管理会话和执行 Agent 循环。

use std::{
    path::PathBuf,
    sync::{Arc, RwLock as StdRwLock},
};

use astrcode_core::{
    AgentProfile, AgentProfileCatalog, AllowAllPolicyEngine, AstrError, HookHandler, PolicyEngine,
    RuntimeHandle, SessionManager,
};
use astrcode_runtime_agent_control::AgentControl;
use astrcode_runtime_agent_loader::{AgentProfileLoader, AgentProfileRegistry};
use astrcode_runtime_agent_loop::{
    AgentLoop, ApprovalBroker, DefaultApprovalBroker, DynProviderFactory,
};
use astrcode_runtime_prompt::{
    LayeredPromptBuilder, PromptDeclaration, default_layered_prompt_builder,
};
use astrcode_runtime_registry::CapabilityRouter;
use astrcode_runtime_session::SessionState;
use astrcode_runtime_skill_loader::SkillCatalog;
use astrcode_storage::session::FileSystemSessionRepository;
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{config::load_config, provider_factory::ConfigFileProviderFactory};

mod agent;
#[cfg(test)]
mod baselines;
mod blocking_bridge;
mod composer;
mod config;
mod execution;
mod lifecycle;
mod loop_surface;
mod observability;
mod service_contract;
mod session;
mod turn;
mod watch;

pub use agent::AgentServiceHandle;
pub(crate) use agent::{DeferredCollaborationExecutor, service_error_to_astr};
pub use composer::ComposerServiceHandle;
pub use config::ConfigServiceHandle;
pub(crate) use execution::DeferredSubAgentExecutor;
pub use execution::{
    AgentExecutionServiceHandle, AgentProfileSummary, ToolExecutionServiceHandle, ToolSummary,
};
pub use lifecycle::LifecycleServiceHandle;
pub use loop_surface::LoopSurfaceServiceHandle;
use observability::RuntimeObservability;
pub use observability::{
    ExecutionDiagnosticsSnapshot, ObservabilityServiceHandle, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
    SubRunExecutionMetricsSnapshot,
};
pub use session::{SessionEventFilter, SessionServiceHandle};
pub use watch::WatchServiceHandle;

use self::loop_surface::{LoopRuntimeDeps, build_agent_loop};
pub use self::service_contract::{
    ComposerOption, ComposerOptionKind, ComposerOptionsRequest, ServiceError, ServiceResult,
    SessionCatalogEvent, SessionEventFilterSpec, SessionEventRecord, SessionHistorySnapshot,
    SessionReplay, SessionReplaySource, SessionViewSnapshot, SubRunEventScope,
    SubRunStatusSnapshot, SubRunStatusSource,
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
    prompt_builder: LayeredPromptBuilder,
    /// LLM Provider 工厂，用于子代理 scoped execution 组装 AgentLoop。
    /// 测试中可通过 `install_test_loop` 注入 StaticProvider。
    factory: DynProviderFactory,
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
    session_load_lock: Arc<Mutex<()>>,
    /// 同一 session 的 durable replay singleflight。
    ///
    /// recent cache miss 时，共享同一次磁盘回放结果，避免并发请求重复回盘。
    replay_fallbacks: DashMap<String, turn::ReplayFallbackFuture>,
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
    /// `spawn` 会反复查询同一个 working dir 的 profile 视图；如果每次都重新扫盘，
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
    /// watch 子边界的内部状态（AtomicBool + JoinHandle）。
    watch_state: watch::WatchState,
    /// lifecycle 子边界的任务注册表（turn + subagent JoinHandle）。
    task_registry: lifecycle::TaskRegistry,
    /// 协作工具的延迟执行器桥（runtime surface 热重载时复用）。
    collaboration_executor: Arc<DeferredCollaborationExecutor>,
}

impl RuntimeService {
    pub fn owner_graph(&self) -> RuntimeOwnerGraph {
        RuntimeOwnerGraph {
            session_owner: "runtime-session",
            execution_owner: "runtime-execution",
            tool_owner: "runtime-agent",
        }
    }

    pub fn composer(self: &Arc<Self>) -> ComposerServiceHandle {
        ComposerServiceHandle::new(Arc::clone(self))
    }

    pub fn config(self: &Arc<Self>) -> ConfigServiceHandle {
        ConfigServiceHandle::new(Arc::clone(self))
    }

    pub fn agent(self: &Arc<Self>) -> AgentServiceHandle {
        AgentServiceHandle {
            runtime: Arc::clone(self),
        }
    }

    pub fn watch(self: &Arc<Self>) -> WatchServiceHandle {
        WatchServiceHandle::new(Arc::clone(self))
    }

    pub fn loop_surface(self: &Arc<Self>) -> LoopSurfaceServiceHandle {
        LoopSurfaceServiceHandle::new(Arc::clone(self))
    }

    pub fn lifecycle(self: &Arc<Self>) -> LifecycleServiceHandle {
        LifecycleServiceHandle::new(Arc::clone(self))
    }

    pub fn observability(self: &Arc<Self>) -> ObservabilityServiceHandle {
        ObservabilityServiceHandle::new(Arc::clone(self))
    }

    fn agent_profile_catalog(&self) -> Arc<dyn AgentProfileCatalog> {
        Arc::new(RuntimeAgentProfileCatalog::new(Arc::clone(
            &self.agent_profiles,
        )))
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
            prompt_builder: default_layered_prompt_builder(),
            factory: Arc::new(ConfigFileProviderFactory),
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
            session_load_lock: Arc::new(Mutex::new(())),
            replay_fallbacks: DashMap::new(),
            observability: Arc::new(RuntimeObservability::default()),
            agent_control,
            agent_loader,
            agent_profiles,
            scoped_agent_profiles: DashMap::new(),
            session_catalog_events,
            shutdown_token: CancellationToken::new(),
            rebuild_lock: Mutex::new(()),
            watch_state: watch::WatchState::new(),
            task_registry: lifecycle::TaskRegistry::new(),
            collaboration_executor: Arc::new(DeferredCollaborationExecutor::default()),
        })
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

    pub fn agent_loader(&self) -> Arc<AgentProfileLoader> {
        Arc::clone(&self.agent_loader)
    }

    pub fn agent_profiles(&self) -> Arc<AgentProfileRegistry> {
        self.agent_profiles
            .read()
            .expect("agent profile registry lock should not be poisoned")
            .clone()
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
        lifecycle::LifecycleService::new(self)
            .shutdown(timeout_secs)
            .await;
        Ok(())
    }
}
