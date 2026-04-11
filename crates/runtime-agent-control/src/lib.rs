//! # Agent 控制平面
//!
//! 提供轻量的 in-memory Agent 注册表，负责：
//! - 分配 agent 实例 ID
//! - 跟踪 parent-child 关系
//! - 对外暴露 spawn / list / cancel / wait
//! - 维护父取消传播
//!
//! 之所以单独拆成 crate，是为了把“控制平面”从“执行引擎”中拿出来：
//! `runtime-agent-loop` 专注一次 turn 如何执行，
//! `runtime-agent-control` 专注多 Agent 生命周期如何被编排和取消。

use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentProfile, AgentStatus, AgentTurnOutcome,
    AstrError, CancelToken, LiveSubRunControlBoundary, SpawnAgentParams, SubRunHandle,
    SubRunResult, SubRunStorageMode, ToolContext,
};
use astrcode_runtime_config::{
    RuntimeConfig, resolve_agent_finalized_retain_limit, resolve_agent_max_concurrent,
    resolve_agent_max_subrun_depth,
};
use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{RwLock, watch};

#[derive(Default)]
struct AgentRegistryState {
    entries: HashMap<String, AgentEntry>,
    agent_index: HashMap<String, String>,
    active_count: usize,
    parent_delivery_queues: HashMap<String, ParentDeliveryQueue>,
}

struct AgentEntry {
    handle: SubRunHandle,
    cancel: CancelToken,
    status_tx: watch::Sender<AgentStatus>,
    parent_agent_id: Option<String>,
    children: BTreeSet<String>,
    finalized_seq: Option<u64>,
    /// 协作消息收件箱。sendAgent / deliverToParent 产出信封存放在此。
    inbox: VecDeque<AgentInboxEnvelope>,
    /// 收件箱版本号，每次 push_inbox 递增，用于 wait_for_inbox 的变化检测。
    inbox_version: watch::Sender<u64>,
    /// 四工具模型的持久生命周期状态（与旧 AgentStatus 正交）。
    /// Pending → Running → Idle → Terminated，完成单轮后不自动终止。
    lifecycle_status: AgentLifecycleStatus,
    /// 最近一轮执行的结束原因。Running 期间为 None，turn 完成后设为 Some。
    last_turn_outcome: Option<AgentTurnOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingParentDelivery {
    pub delivery_id: String,
    pub parent_session_id: String,
    pub parent_turn_id: String,
    pub notification: astrcode_core::ChildSessionNotification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingParentDeliveryState {
    Queued,
    WakingParent,
}

#[derive(Debug, Clone)]
struct PendingParentDeliveryEntry {
    delivery: PendingParentDelivery,
    state: PendingParentDeliveryState,
}

#[derive(Default)]
struct ParentDeliveryQueue {
    deliveries: VecDeque<PendingParentDeliveryEntry>,
    known_delivery_ids: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentControlError {
    #[error("parent agent '{agent_id}' does not exist")]
    ParentAgentNotFound { agent_id: String },
    #[error("agent depth {current} exceeds max depth {max}")]
    MaxDepthExceeded { current: usize, max: usize },
    #[error("active agent count {current} exceeds max concurrent {max}")]
    MaxConcurrentExceeded { current: usize, max: usize },
}

/// Agent 控制平面主句柄。
#[derive(Clone)]
pub struct AgentControl {
    next_id: Arc<AtomicU64>,
    next_finalized_seq: Arc<AtomicU64>,
    max_depth: usize,
    max_concurrent: usize,
    finalized_retain_limit: usize,
    state: Arc<RwLock<AgentRegistryState>>,
}

pub trait AgentProfileSource: Send + Sync {
    fn list_profiles(&self) -> Vec<AgentProfile>;
}

#[derive(Clone)]
pub struct StaticAgentProfileSource {
    profiles: Vec<AgentProfile>,
}

impl StaticAgentProfileSource {
    pub fn new(profiles: Vec<AgentProfile>) -> Self {
        Self { profiles }
    }
}

impl AgentProfileSource for StaticAgentProfileSource {
    fn list_profiles(&self) -> Vec<AgentProfile> {
        self.profiles.clone()
    }
}

/// `runtime-agent-control` 对外暴露的 live 控制面。
///
/// live registry 负责 handle/cancel 真相，profile 列表由外部 profile catalog 注入，
/// 避免控制平面反向依赖 loader/runtime façade。
#[derive(Clone)]
pub struct LiveSubRunControl<P> {
    control: AgentControl,
    profiles: P,
}

impl<P> LiveSubRunControl<P> {
    pub fn new(control: AgentControl, profiles: P) -> Self {
        Self { control, profiles }
    }

    pub fn control(&self) -> &AgentControl {
        &self.control
    }
}

#[async_trait]
impl<P> LiveSubRunControlBoundary for LiveSubRunControl<P>
where
    P: AgentProfileSource,
{
    async fn get_subrun_handle(
        &self,
        _session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<Option<SubRunHandle>, AstrError> {
        Ok(self.control.get(sub_run_id).await)
    }

    async fn cancel_subrun(
        &self,
        _session_id: &str,
        sub_run_id: &str,
    ) -> std::result::Result<(), AstrError> {
        // 故意忽略：取消子运行时的错误不应阻断父级流程
        let _ = self.control.cancel(sub_run_id).await;
        Ok(())
    }

    /// 控制平面本身不持有执行引擎，launch_subagent 应由 runtime service 层实现。
    ///
    /// 此 impl 仅满足 trait 约束；实际调用不会到达这里，因为 runtime service
    /// 会使用自己的 `LiveSubRunControlBoundary` impl 来组合 control + execution。
    async fn launch_subagent(
        &self,
        _params: SpawnAgentParams,
        _ctx: &ToolContext,
    ) -> std::result::Result<SubRunResult, AstrError> {
        Err(AstrError::Internal(
            "launch_subagent must be called through the runtime service LiveSubRunControlBoundary \
             impl, not the bare control-plane wrapper"
                .to_string(),
        ))
    }

    async fn list_profiles(&self) -> std::result::Result<Vec<AgentProfile>, AstrError> {
        Ok(self.profiles.list_profiles())
    }
}

impl Default for AgentControl {
    fn default() -> Self {
        Self::from_config(&RuntimeConfig::default())
    }
}

impl AgentControl {
    /// 从 RuntimeConfig 构建 AgentControl，读取 agent 子分组配置。
    ///
    /// 未设置的字段会回退到内置默认值。
    pub fn from_config(runtime: &RuntimeConfig) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            next_finalized_seq: Arc::new(AtomicU64::new(0)),
            max_depth: resolve_agent_max_subrun_depth(runtime.agent.as_ref()),
            max_concurrent: resolve_agent_max_concurrent(runtime.agent.as_ref()),
            finalized_retain_limit: resolve_agent_finalized_retain_limit(runtime.agent.as_ref()),
            state: Arc::new(RwLock::new(AgentRegistryState::default())),
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_limits(max_depth: usize, max_concurrent: usize, finalized_retain_limit: usize) -> Self {
        Self {
            max_depth,
            max_concurrent,
            finalized_retain_limit,
            ..Self::default()
        }
    }

    /// 注册一个新的子 Agent 实例。
    ///
    /// 只建立控制面，不假设 spawn 立刻意味着开始执行，因此初始状态为 Pending。
    pub async fn spawn(
        &self,
        profile: &AgentProfile,
        session_id: impl Into<String>,
        parent_turn_id: String,
        parent_agent_id: Option<String>,
    ) -> Result<SubRunHandle, AgentControlError> {
        self.spawn_with_storage(
            profile,
            session_id,
            None,
            parent_turn_id,
            parent_agent_id,
            SubRunStorageMode::SharedSession,
        )
        .await
    }

    /// 注册一个新的子 Agent / 子会话实例，并显式指定存储模式。
    pub async fn spawn_with_storage(
        &self,
        profile: &AgentProfile,
        session_id: impl Into<String>,
        child_session_id: Option<String>,
        parent_turn_id: String,
        parent_agent_id: Option<String>,
        storage_mode: SubRunStorageMode,
    ) -> Result<SubRunHandle, AgentControlError> {
        let mut state = self.state.write().await;
        let depth = match parent_agent_id.as_ref() {
            Some(parent_agent_id) => {
                let Some(parent_sub_run_id) = state.agent_index.get(parent_agent_id) else {
                    return Err(AgentControlError::ParentAgentNotFound {
                        agent_id: parent_agent_id.clone(),
                    });
                };
                let Some(parent) = state.entries.get(parent_sub_run_id) else {
                    return Err(AgentControlError::ParentAgentNotFound {
                        agent_id: parent_agent_id.clone(),
                    });
                };
                parent.handle.depth + 1
            },
            None => 1,
        };
        if depth > self.max_depth {
            return Err(AgentControlError::MaxDepthExceeded {
                current: depth,
                max: self.max_depth,
            });
        }
        if state.active_count >= self.max_concurrent {
            return Err(AgentControlError::MaxConcurrentExceeded {
                current: state.active_count,
                max: self.max_concurrent,
            });
        }
        // 只有在父节点校验通过后才分配新 ID，避免失败的 spawn 留下无意义的编号空洞。
        let next_id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let agent_id = format!("agent-{next_id}");
        let sub_run_id = format!("subrun-{next_id}");
        let session_id = session_id.into();
        let handle = SubRunHandle {
            sub_run_id: sub_run_id.clone(),
            agent_id: agent_id.clone(),
            session_id,
            child_session_id,
            depth,
            parent_turn_id,
            parent_agent_id: parent_agent_id.clone(),
            agent_profile: profile.id.clone(),
            storage_mode,
            status: AgentStatus::Pending,
        };
        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(handle.status);
        state.entries.insert(
            sub_run_id.clone(),
            AgentEntry {
                handle: handle.clone(),
                cancel,
                status_tx,
                parent_agent_id: parent_agent_id.clone(),
                children: BTreeSet::new(),
                finalized_seq: None,
                inbox: VecDeque::new(),
                inbox_version: watch::channel(0).0,
                lifecycle_status: AgentLifecycleStatus::Pending,
                last_turn_outcome: None,
            },
        );
        state.agent_index.insert(agent_id, sub_run_id.clone());
        state.active_count += 1;
        if let Some(parent_agent_id) = parent_agent_id {
            if let Some(parent_sub_run_id) = state.agent_index.get(&parent_agent_id).cloned() {
                if let Some(parent) = state.entries.get_mut(&parent_sub_run_id) {
                    parent.children.insert(sub_run_id);
                }
            }
        }
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        Ok(handle)
    }

    /// 注册根 Agent 到控制树。
    ///
    /// 四工具模型要求根 Agent 也成为一等控制对象，
    /// 这样 child 可以通过 `send(parentId, ...)` 向根发送消息，
    /// `observe` 也可以沿着统一父子树进行权限校验。
    ///
    /// 根 Agent 深度为 0，无父节点，生命周期初始为 Running（根已在执行中）。
    pub async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<SubRunHandle, AgentControlError> {
        let mut state = self.state.write().await;
        // 如果该 agent 已注册，直接返回现有句柄（幂等）
        if let Some(existing_key) = state.agent_index.get(&agent_id) {
            if let Some(entry) = state.entries.get(existing_key) {
                return Ok(entry.handle.clone());
            }
        }
        // 根 agent 没有真实 sub_run_id，使用 agent_id 等价
        let sub_run_id = format!("root-{agent_id}");
        let handle = SubRunHandle {
            sub_run_id: sub_run_id.clone(),
            agent_id: agent_id.clone(),
            session_id,
            child_session_id: None,
            depth: 0,
            parent_turn_id: String::new(),
            parent_agent_id: None,
            agent_profile: profile_id,
            storage_mode: SubRunStorageMode::SharedSession,
            status: AgentStatus::Running,
        };
        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(handle.status);
        state.entries.insert(
            sub_run_id.clone(),
            AgentEntry {
                handle: handle.clone(),
                cancel,
                status_tx,
                parent_agent_id: None,
                children: BTreeSet::new(),
                finalized_seq: None,
                inbox: VecDeque::new(),
                inbox_version: watch::channel(0).0,
                lifecycle_status: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
            },
        );
        state.agent_index.insert(agent_id, sub_run_id);
        state.active_count += 1;
        Ok(handle)
    }

    // ── 生命周期与轮次结果（四工具模型） ──────────────────────────────

    /// 读取 agent 的持久生命周期状态。
    pub async fn get_lifecycle(&self, id: &str) -> Option<AgentLifecycleStatus> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, id)?;
        state.entries.get(key).map(|e| e.lifecycle_status)
    }

    /// 读取 agent 的最近一轮执行结果。
    pub async fn get_turn_outcome(&self, id: &str) -> Option<Option<AgentTurnOutcome>> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, id)?;
        state.entries.get(key).map(|e| e.last_turn_outcome)
    }

    /// 更新 agent 的持久生命周期状态。
    ///
    /// 状态迁移规则由调用方保证合法性（Pending→Running→Idle→Terminated），
    /// 此方法不做状态机校验，只做原子写入。
    pub async fn set_lifecycle(&self, id: &str, new_status: AgentLifecycleStatus) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.lifecycle_status = new_status;
        let projected_status =
            lifecycle_to_legacy_status(entry.lifecycle_status, entry.last_turn_outcome)?;
        entry.handle.status = projected_status;
        entry.status_tx.send_replace(projected_status);
        Some(())
    }

    /// 更新 agent 的最近一轮执行结果。
    ///
    /// 在 turn 完成（无论是正常完成还是失败）时调用，
    /// 同时将 lifecycle 从 Running 推进到 Idle。
    pub async fn complete_turn(
        &self,
        id: &str,
        outcome: AgentTurnOutcome,
    ) -> Option<AgentLifecycleStatus> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.last_turn_outcome = Some(outcome);
        entry.lifecycle_status = AgentLifecycleStatus::Idle;
        let projected_status =
            lifecycle_to_legacy_status(entry.lifecycle_status, entry.last_turn_outcome)?;
        entry.handle.status = projected_status;
        entry.status_tx.send_replace(projected_status);
        Some(entry.lifecycle_status)
    }

    /// 列出当前已注册的 Agent。
    pub async fn list(&self) -> Vec<SubRunHandle> {
        let state = self.state.read().await;
        let mut handles = state
            .entries
            .values()
            .map(|entry| entry.handle.clone())
            .collect::<Vec<_>>();
        handles.sort_by(|left, right| left.sub_run_id.cmp(&right.sub_run_id));
        handles
    }

    /// 查询单个 Agent。
    pub async fn get(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?;
        state.entries.get(key).map(|entry| entry.handle.clone())
    }

    /// 获取某个 Agent 的取消令牌，供真正的执行器复用。
    pub async fn cancel_token(&self, sub_run_or_agent_id: &str) -> Option<CancelToken> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?;
        state.entries.get(key).map(|entry| entry.cancel.clone())
    }

    /// 标记 Agent 已开始运行。
    pub async fn mark_running(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.update_status(sub_run_or_agent_id, AgentStatus::Running)
            .await
    }

    /// 标记 Agent 正常完成。
    pub async fn mark_completed(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.update_status(sub_run_or_agent_id, AgentStatus::Completed)
            .await
    }

    /// 标记 Agent 执行失败。
    pub async fn mark_failed(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.update_status(sub_run_or_agent_id, AgentStatus::Failed)
            .await
    }

    /// 为已终态的 Agent 创建新的执行实例。
    ///
    /// 只有 Completed/Failed/Cancelled 状态的 Agent 可以被恢复。
    /// 恢复不会篡改旧执行实例，而是为同一个 agent mint 一个新的 `sub_run_id`，
    /// 这样 child session 可以沿用稳定身份，同时把新的执行实例显式暴露出来。
    pub async fn resume(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();

        // 先检查状态是否可恢复
        if !state
            .entries
            .get(&key)
            .is_some_and(|entry| entry.handle.status.is_final())
        {
            return None;
        }

        let old_entry = state.entries.get(&key)?;
        let old_handle = old_entry.handle.clone();
        let parent_agent_id = old_entry.parent_agent_id.clone();
        let children = old_entry.children.clone();

        let next_id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let new_sub_run_id = format!("subrun-{next_id}");
        let mut new_handle = old_handle.clone();
        new_handle.sub_run_id = new_sub_run_id.clone();
        new_handle.status = AgentStatus::Running;

        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(new_handle.status);
        let inbox_version = watch::channel(0).0;

        state.active_count += 1;
        state.entries.insert(
            new_sub_run_id.clone(),
            AgentEntry {
                handle: new_handle.clone(),
                cancel,
                status_tx,
                parent_agent_id: parent_agent_id.clone(),
                children: children.clone(),
                finalized_seq: None,
                inbox: VecDeque::new(),
                inbox_version,
                lifecycle_status: AgentLifecycleStatus::Pending,
                last_turn_outcome: None,
            },
        );
        state
            .agent_index
            .insert(new_handle.agent_id.clone(), new_sub_run_id.clone());

        if let Some(parent_agent_id) = parent_agent_id {
            if let Some(parent_sub_run_id) = state.agent_index.get(&parent_agent_id).cloned() {
                if let Some(parent) = state.entries.get_mut(&parent_sub_run_id) {
                    parent.children.remove(&key);
                    parent.children.insert(new_sub_run_id);
                }
            }
        }

        Some(new_handle)
    }

    /// 取消指定 Agent，并级联取消其子树。
    pub async fn cancel(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        let mut state = self.state.write().await;
        let mut visited = HashSet::new();
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let handle = cancel_tree(&mut state, &key, &mut visited, &self.next_finalized_seq);
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        handle
    }

    /// 按父 turn 取消所有子 Agent。
    ///
    /// 这里显式按 parent turn 做传播，而不是把取消关系隐式塞进 `CancelToken`，
    /// 因为控制平面只关心“谁挂在谁下面”，不应该把执行器内部任务结构反向泄漏进来。
    pub async fn cancel_for_parent_turn(&self, parent_turn_id: &str) -> Vec<SubRunHandle> {
        let mut state = self.state.write().await;
        let mut roots = state
            .entries
            .values()
            .filter(|entry| entry.handle.parent_turn_id == parent_turn_id)
            .filter(|entry| {
                !entry
                    .parent_agent_id
                    .as_ref()
                    .is_some_and(|parent_agent_id| {
                        state
                            .agent_index
                            .get(parent_agent_id)
                            .and_then(|parent_sub_run_id| state.entries.get(parent_sub_run_id))
                            .is_some_and(|parent| parent.handle.parent_turn_id == parent_turn_id)
                    })
            })
            .map(|entry| entry.handle.sub_run_id.clone())
            .collect::<Vec<_>>();
        roots.sort();

        let mut cancelled = Vec::new();
        let mut visited = HashSet::new();
        for agent_id in roots {
            cancel_tree_collect(
                &mut state,
                &agent_id,
                &mut visited,
                &mut cancelled,
                &self.next_finalized_seq,
            );
        }
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        cancelled
    }

    /// 等待 Agent 到达终态。
    pub async fn wait(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        let mut status_rx = {
            let state = self.state.read().await;
            let key = resolve_entry_key(&state, sub_run_or_agent_id)?;
            state.entries.get(key)?.status_tx.subscribe()
        };

        loop {
            let current = *status_rx.borrow_and_update();
            if current.is_final() {
                return self.get(sub_run_or_agent_id).await;
            }
            if status_rx.changed().await.is_err() {
                return self.get(sub_run_or_agent_id).await;
            }
        }
    }

    /// 向 Agent 收件箱推送一封信封。
    ///
    /// 若目标 agent 不存在则返回 None。
    /// 推送后会递增收件箱版本号，唤醒正在 wait_for_inbox 的调用方。
    pub async fn push_inbox(
        &self,
        sub_run_or_agent_id: &str,
        envelope: AgentInboxEnvelope,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.inbox.push_back(envelope);
        // 递增版本号唤醒 wait_for_inbox
        let current = *entry.inbox_version.borrow();
        entry.inbox_version.send_replace(current + 1);
        Some(())
    }

    /// 排空 Agent 收件箱，返回所有待处理信封。
    pub async fn drain_inbox(&self, sub_run_or_agent_id: &str) -> Option<Vec<AgentInboxEnvelope>> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        let envelopes: Vec<_> = entry.inbox.drain(..).collect();
        Some(envelopes)
    }

    /// 等待 Agent 收件箱收到新信封。
    ///
    /// 若目标不存在或 agent 已到达终态则立即返回 None。
    /// 否则阻塞直到收件箱版本号变化（即有新信封到达）。
    pub async fn wait_for_inbox(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        let mut inbox_rx = {
            let state = self.state.read().await;
            let key = resolve_entry_key(&state, sub_run_or_agent_id)?;
            state.entries.get(key)?.inbox_version.subscribe()
        };

        loop {
            let handle = self.get(sub_run_or_agent_id).await?;
            // 如果 agent 已终态，直接返回当前 handle
            if handle.status.is_final() {
                return Some(handle);
            }
            // 如果收件箱非空，返回当前 handle
            {
                let state = self.state.read().await;
                if let Some(key) = resolve_entry_key(&state, sub_run_or_agent_id) {
                    if let Some(entry) = state.entries.get(key) {
                        if !entry.inbox.is_empty() {
                            return Some(handle);
                        }
                    }
                }
            }
            // 等待收件箱版本号变化
            if inbox_rx.changed().await.is_err() {
                return self.get(sub_run_or_agent_id).await;
            }
        }
    }

    /// 向父会话排入一个待消费的 child terminal delivery。
    ///
    /// 以 `delivery_id` 做幂等去重；重复交付会被忽略，保持原队列顺序不变。
    /// 队列变更全部限制在单个写锁临界区内完成，避免把异步工作带进锁作用域。
    pub async fn enqueue_parent_delivery(
        &self,
        parent_session_id: impl Into<String>,
        parent_turn_id: impl Into<String>,
        notification: astrcode_core::ChildSessionNotification,
    ) -> bool {
        let parent_session_id = parent_session_id.into();
        let delivery_id = notification.notification_id.clone();
        let mut state = self.state.write().await;
        let queue = state
            .parent_delivery_queues
            .entry(parent_session_id.clone())
            .or_default();
        if !queue.known_delivery_ids.insert(delivery_id.clone()) {
            return false;
        }
        queue.deliveries.push_back(PendingParentDeliveryEntry {
            delivery: PendingParentDelivery {
                delivery_id,
                parent_session_id,
                parent_turn_id: parent_turn_id.into(),
                notification,
            },
            state: PendingParentDeliveryState::Queued,
        });
        true
    }

    /// 查看并锁定当前父会话最前面的待消费交付。
    ///
    /// 只有队头处于 `Queued` 状态时才会返回，并原子地标记为 `WakingParent`，
    /// 避免并发唤醒时重复消费同一条交付。
    pub async fn checkout_parent_delivery(
        &self,
        parent_session_id: &str,
    ) -> Option<PendingParentDelivery> {
        let mut state = self.state.write().await;
        let queue = state.parent_delivery_queues.get_mut(parent_session_id)?;
        let entry = queue.deliveries.front_mut()?;
        if !matches!(entry.state, PendingParentDeliveryState::Queued) {
            return None;
        }
        entry.state = PendingParentDeliveryState::WakingParent;
        Some(entry.delivery.clone())
    }

    /// 以 turn-start snapshot drain 的方式锁定一个父级交付批次。
    ///
    /// 批次规则：
    /// - 只从队头开始抓取连续的 `Queued` 项
    /// - 批内 delivery 必须属于同一个直接父 agent，避免一次 wake turn 混入不同 owner
    /// - 抓取后统一标记为 `WakingParent`，后续必须整体 consume 或 requeue
    pub async fn checkout_parent_delivery_batch(
        &self,
        parent_session_id: &str,
    ) -> Option<Vec<PendingParentDelivery>> {
        let mut state = self.state.write().await;
        let queue = state.parent_delivery_queues.get_mut(parent_session_id)?;
        let first = queue.deliveries.front()?;
        if !matches!(first.state, PendingParentDeliveryState::Queued) {
            return None;
        }

        let target_parent_agent_id = first
            .delivery
            .notification
            .child_ref
            .parent_agent_id
            .clone();
        let mut batch_len = 0usize;
        for entry in &queue.deliveries {
            if !matches!(entry.state, PendingParentDeliveryState::Queued) {
                break;
            }
            if entry.delivery.notification.child_ref.parent_agent_id != target_parent_agent_id {
                break;
            }
            batch_len += 1;
        }

        if batch_len == 0 {
            return None;
        }

        let mut deliveries = Vec::with_capacity(batch_len);
        for entry in queue.deliveries.iter_mut().take(batch_len) {
            entry.state = PendingParentDeliveryState::WakingParent;
            deliveries.push(entry.delivery.clone());
        }
        Some(deliveries)
    }

    /// 将正在唤醒中的交付标记回 `Queued`，用于父会话繁忙或启动失败后的重试。
    pub async fn requeue_parent_delivery(
        &self,
        parent_session_id: &str,
        delivery_id: &str,
    ) -> bool {
        let mut state = self.state.write().await;
        let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
            return false;
        };
        let Some(entry) = queue
            .deliveries
            .iter_mut()
            .find(|entry| entry.delivery.delivery_id == delivery_id)
        else {
            return false;
        };
        entry.state = PendingParentDeliveryState::Queued;
        true
    }

    /// 将一批正在唤醒中的交付重新标记为 `Queued`。
    pub async fn requeue_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> usize {
        let mut state = self.state.write().await;
        let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
            return 0;
        };

        let target_ids = delivery_ids.iter().collect::<HashSet<_>>();
        let mut updated = 0usize;
        for entry in &mut queue.deliveries {
            if target_ids.contains(&entry.delivery.delivery_id) {
                entry.state = PendingParentDeliveryState::Queued;
                updated += 1;
            }
        }
        updated
    }

    /// 确认最前面的交付已经被父 turn 消费，并将其从缓冲中移除。
    pub async fn consume_parent_delivery(
        &self,
        parent_session_id: &str,
        delivery_id: &str,
    ) -> bool {
        let mut state = self.state.write().await;
        let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
            return false;
        };
        let Some(front) = queue.deliveries.front() else {
            return false;
        };
        if front.delivery.delivery_id != delivery_id {
            return false;
        }
        let removed = queue.deliveries.pop_front();
        if let Some(removed) = removed {
            queue
                .known_delivery_ids
                .remove(&removed.delivery.delivery_id);
        }
        if queue.deliveries.is_empty() {
            state.parent_delivery_queues.remove(parent_session_id);
        }
        true
    }

    /// 确认一整个交付批次已经被父 turn 消费，并按 FIFO 从队头移除。
    pub async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool {
        let mut state = self.state.write().await;
        let Some(queue) = state.parent_delivery_queues.get_mut(parent_session_id) else {
            return false;
        };

        for delivery_id in delivery_ids {
            let Some(front) = queue.deliveries.front() else {
                return false;
            };
            if front.delivery.delivery_id != *delivery_id {
                return false;
            }
            let removed = queue.deliveries.pop_front();
            if let Some(removed) = removed {
                queue
                    .known_delivery_ids
                    .remove(&removed.delivery.delivery_id);
            }
        }

        if queue.deliveries.is_empty() {
            state.parent_delivery_queues.remove(parent_session_id);
        }
        true
    }

    pub async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        let state = self.state.read().await;
        state
            .parent_delivery_queues
            .get(parent_session_id)
            .map(|queue| queue.deliveries.len())
            .unwrap_or(0)
    }

    async fn update_status(
        &self,
        sub_run_or_agent_id: &str,
        next_status: AgentStatus,
    ) -> Option<SubRunHandle> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let handle = update_status_locked(&mut state, &key, next_status, &self.next_finalized_seq);
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        handle
    }

    /// 终止指定 agent 及其整棵子树（四工具模型 close 语义）。
    ///
    /// 与 `cancel` 不同，terminate 使用 `AgentLifecycleStatus::Terminated`，
    /// 而不是旧 `AgentStatus::Cancelled`。四工具模型要求 `close` 后 agent
    /// 进入 `Terminated` 生命周期，且后续 `send` 被拒绝。
    ///
    /// 终止过程中：
    /// 1. 对每个节点设置 lifecycle = Terminated
    /// 2. 触发 cancel token 以中断正在运行的 turn
    /// 3. 级联到所有后代
    pub async fn terminate_subtree(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        let mut state = self.state.write().await;
        let mut visited = HashSet::new();
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let handle = terminate_tree(&mut state, &key, &mut visited, &self.next_finalized_seq);
        let terminated_agent_ids = visited
            .iter()
            .filter_map(|entry_key| state.entries.get(entry_key))
            .map(|entry| entry.handle.agent_id.clone())
            .collect::<HashSet<_>>();
        discard_parent_deliveries_locked(&mut state, &terminated_agent_ids);
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        handle
    }

    /// 收集指定 agent 子树的所有 agent handle（不含自身）。
    ///
    /// 从给定 agent 出发，递归查找其所有后代 agent，
    /// 用于层级协作场景下的子树隔离和级联关闭范围确认。
    pub async fn collect_subtree_handles(&self, sub_run_or_agent_id: &str) -> Vec<SubRunHandle> {
        let state = self.state.read().await;
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::new();

        // 从直接子节点开始，不包含自身
        if let Some(key) = resolve_entry_key(&state, sub_run_or_agent_id) {
            if let Some(entry) = state.entries.get(key) {
                for child_sub_run_id in &entry.children {
                    queue.push_back(child_sub_run_id.clone());
                }
            }
        }

        while let Some(child_key) = queue.pop_front() {
            if let Some(entry) = state.entries.get(&child_key) {
                result.push(entry.handle.clone());
                for grandchild in &entry.children {
                    queue.push_back(grandchild.clone());
                }
            }
        }

        result
    }

    /// 获取指定 agent 的祖先链（从自身到根节点的路径）。
    ///
    /// 返回从自身开始向上到根的所有 agent handle，
    /// 用于 deliverToParent 验证直接父路由。
    pub async fn ancestor_chain(&self, sub_run_or_agent_id: &str) -> Vec<SubRunHandle> {
        let state = self.state.read().await;
        let mut chain = Vec::new();

        // 先加入自身
        if let Some(key) = resolve_entry_key(&state, sub_run_or_agent_id) {
            if let Some(entry) = state.entries.get(key) {
                chain.push(entry.handle.clone());
                // 向上遍历父节点
                let mut current_parent = entry.parent_agent_id.clone();
                while let Some(parent_agent_id) = current_parent {
                    if let Some(parent_key) = state.agent_index.get(&parent_agent_id) {
                        if let Some(parent_entry) = state.entries.get(parent_key) {
                            chain.push(parent_entry.handle.clone());
                            current_parent = parent_entry.parent_agent_id.clone();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        }

        chain
    }
}

fn resolve_entry_key<'a>(
    state: &'a AgentRegistryState,
    sub_run_or_agent_id: &'a str,
) -> Option<&'a str> {
    if state.entries.contains_key(sub_run_or_agent_id) {
        return Some(sub_run_or_agent_id);
    }
    state
        .agent_index
        .get(sub_run_or_agent_id)
        .map(String::as_str)
}

fn lifecycle_to_legacy_status(
    lifecycle: AgentLifecycleStatus,
    last_turn_outcome: Option<AgentTurnOutcome>,
) -> Option<AgentStatus> {
    match lifecycle {
        AgentLifecycleStatus::Pending => Some(AgentStatus::Pending),
        AgentLifecycleStatus::Running => Some(AgentStatus::Running),
        AgentLifecycleStatus::Idle => match last_turn_outcome {
            Some(AgentTurnOutcome::Completed) => Some(AgentStatus::Completed),
            Some(AgentTurnOutcome::Failed) => Some(AgentStatus::Failed),
            Some(AgentTurnOutcome::Cancelled) => Some(AgentStatus::Cancelled),
            Some(AgentTurnOutcome::TokenExceeded) => Some(AgentStatus::TokenExceeded),
            None => None,
        },
        AgentLifecycleStatus::Terminated => Some(AgentStatus::Cancelled),
    }
}

fn update_status_locked(
    state: &mut AgentRegistryState,
    agent_id: &str,
    next_status: AgentStatus,
    next_finalized_seq: &AtomicU64,
) -> Option<SubRunHandle> {
    let entry = state.entries.get_mut(agent_id)?;
    if entry.handle.status.is_final() {
        return Some(entry.handle.clone());
    }
    let was_active = !entry.handle.status.is_final();
    entry.handle.status = next_status;
    // 为什么同步推进 lifecycle：避免双真相源。旧 AgentStatus 驱动取消/并发槽，
    // 新 lifecycle 驱动四工具模型的 Idle/终止语义。两者必须在同一写锁内同步，
    // 否则 observe 看到的 lifecycle 和 cancel 看到的 status 会对不上。
    match next_status {
        AgentStatus::Running => {
            entry.lifecycle_status = AgentLifecycleStatus::Running;
        },
        AgentStatus::Completed => {
            entry.lifecycle_status = AgentLifecycleStatus::Idle;
            entry.last_turn_outcome = Some(AgentTurnOutcome::Completed);
        },
        AgentStatus::Cancelled => {
            entry.lifecycle_status = AgentLifecycleStatus::Terminated;
            entry.last_turn_outcome = Some(AgentTurnOutcome::Cancelled);
        },
        AgentStatus::Failed => {
            entry.lifecycle_status = AgentLifecycleStatus::Terminated;
            entry.last_turn_outcome = Some(AgentTurnOutcome::Failed);
        },
        AgentStatus::TokenExceeded => {
            entry.lifecycle_status = AgentLifecycleStatus::Idle;
            entry.last_turn_outcome = Some(AgentTurnOutcome::TokenExceeded);
        },
        AgentStatus::Pending => {},
    }
    if next_status.is_final() {
        entry.finalized_seq = Some(next_finalized_seq.fetch_add(1, Ordering::SeqCst));
        // 这里在终态瞬间释放并发槽位，确保新的子 Agent 能及时获取资源；
        // 同时保留终态 handle 供 UI / replay 查询，不把”是否仍可见”与”是否占并发”混为一谈。
        if was_active {
            state.active_count = state.active_count.saturating_sub(1);
        }
    }
    entry.status_tx.send_replace(next_status);
    if matches!(next_status, AgentStatus::Cancelled) {
        entry.cancel.cancel();
    }
    Some(entry.handle.clone())
}

fn cancel_tree(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    next_finalized_seq: &AtomicU64,
) -> Option<SubRunHandle> {
    if !visited.insert(agent_id.to_string()) {
        return state
            .entries
            .get(agent_id)
            .map(|entry| entry.handle.clone());
    }

    let children = state
        .entries
        .get(agent_id)
        .map(|entry| entry.children.iter().cloned().collect::<Vec<_>>())?;

    // 先取消当前节点，再取消子节点，确保父级状态先可见。
    let handle = update_status_locked(state, agent_id, AgentStatus::Cancelled, next_finalized_seq)?;
    for child_id in children {
        // 故意忽略：递归取消子节点，单个失败不阻断其余节点
        let _ = cancel_tree(state, &child_id, visited, next_finalized_seq);
    }
    Some(handle)
}

/// 四工具模型的 subtree terminate 实现。
///
/// 与 `cancel_tree` 使用旧 `AgentStatus::Cancelled` 不同，
/// terminate 设置 `lifecycle_status = Terminated` 并触发 cancel token，
/// 同时释放并发槽位。子 agent 在 Terminated 后拒收任何新 send。
fn terminate_tree(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    next_finalized_seq: &AtomicU64,
) -> Option<SubRunHandle> {
    if !visited.insert(agent_id.to_string()) {
        return state
            .entries
            .get(agent_id)
            .map(|entry| entry.handle.clone());
    }

    let children = state
        .entries
        .get(agent_id)
        .map(|entry| entry.children.iter().cloned().collect::<Vec<_>>())?;

    let entry = state.entries.get_mut(agent_id)?;
    let was_active = !entry.handle.status.is_final();

    // 设置四工具生命周期为 Terminated
    entry.lifecycle_status = AgentLifecycleStatus::Terminated;
    // 同步旧 status 以维持 cancel_token 和并发槽位语义
    entry.handle.status = AgentStatus::Cancelled;
    entry.last_turn_outcome = Some(AgentTurnOutcome::Cancelled);
    entry.inbox.clear();
    entry.finalized_seq = Some(next_finalized_seq.fetch_add(1, Ordering::SeqCst));
    if was_active {
        state.active_count = state.active_count.saturating_sub(1);
    }
    entry.status_tx.send_replace(AgentStatus::Cancelled);
    let current_inbox_version = *entry.inbox_version.borrow();
    entry.inbox_version.send_replace(current_inbox_version + 1);
    // 触发 cancel token 以中断正在运行的 turn
    entry.cancel.cancel();

    let handle = entry.handle.clone();
    for child_id in children {
        let _ = terminate_tree(state, &child_id, visited, next_finalized_seq);
    }
    Some(handle)
}

fn cancel_tree_collect(
    state: &mut AgentRegistryState,
    agent_id: &str,
    visited: &mut HashSet<String>,
    cancelled: &mut Vec<SubRunHandle>,
    next_finalized_seq: &AtomicU64,
) {
    if !visited.insert(agent_id.to_string()) {
        return;
    }

    let Some(children) = state
        .entries
        .get(agent_id)
        .map(|entry| entry.children.iter().cloned().collect::<Vec<_>>())
    else {
        return;
    };

    let status_changed = state
        .entries
        .get(agent_id)
        .is_some_and(|entry| !entry.handle.status.is_final());
    if let Some(handle) =
        update_status_locked(state, agent_id, AgentStatus::Cancelled, next_finalized_seq)
    {
        // 只有真实发生状态迁移时才对外报告取消，避免把已终态节点误记为“本次被取消”。
        if status_changed {
            cancelled.push(handle);
        }
    }
    for child_id in children {
        cancel_tree_collect(state, &child_id, visited, cancelled, next_finalized_seq);
    }
}

fn discard_parent_deliveries_locked(
    state: &mut AgentRegistryState,
    terminated_agent_ids: &HashSet<String>,
) -> usize {
    if terminated_agent_ids.is_empty() {
        return 0;
    }

    let mut removed_count = 0usize;
    let mut empty_sessions = Vec::new();
    for (session_id, queue) in &mut state.parent_delivery_queues {
        let mut retained = VecDeque::new();
        let mut removed_delivery_ids = Vec::new();

        while let Some(entry) = queue.deliveries.pop_front() {
            if terminated_agent_ids.contains(&entry.delivery.notification.child_ref.agent_id) {
                removed_delivery_ids.push(entry.delivery.delivery_id.clone());
            } else {
                retained.push_back(entry);
            }
        }

        for delivery_id in &removed_delivery_ids {
            queue.known_delivery_ids.remove(delivery_id);
        }
        removed_count += removed_delivery_ids.len();
        queue.deliveries = retained;

        if queue.deliveries.is_empty() {
            empty_sessions.push(session_id.clone());
        }
    }

    for session_id in empty_sessions {
        state.parent_delivery_queues.remove(&session_id);
    }

    removed_count
}

fn prune_finalized_agents_locked(state: &mut AgentRegistryState, finalized_retain_limit: usize) {
    if finalized_retain_limit == usize::MAX {
        return;
    }

    loop {
        let mut finalized_leaf_agents = state
            .entries
            .iter()
            .filter_map(|(agent_id, entry)| {
                entry
                    .finalized_seq
                    .filter(|_| entry.children.is_empty())
                    .map(|seq| (seq, agent_id.clone(), entry.parent_agent_id.clone()))
            })
            .collect::<Vec<_>>();
        if finalized_leaf_agents.len() <= finalized_retain_limit {
            break;
        }

        finalized_leaf_agents.sort_by_key(|(seq, agent_id, _)| (*seq, agent_id.clone()));
        let Some((_, agent_id, parent_agent_id)) = finalized_leaf_agents.into_iter().next() else {
            break;
        };

        // 先从父节点摘链，再删除当前终态 leaf，避免留下悬挂 child 引用。
        if let Some(parent_agent_id) = parent_agent_id {
            if let Some(parent_sub_run_id) = state.agent_index.get(&parent_agent_id).cloned() {
                if let Some(parent) = state.entries.get_mut(&parent_sub_run_id) {
                    parent.children.remove(&agent_id);
                }
            }
        }
        if let Some(entry) = state.entries.remove(&agent_id) {
            state.agent_index.remove(&entry.handle.agent_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use astrcode_core::{
        AgentInboxEnvelope, AgentLifecycleStatus, AgentMode, AgentProfile, AgentStatus,
        AgentTurnOutcome, ChildAgentRef, ChildSessionLineageKind, ChildSessionNotification,
        ChildSessionNotificationKind, LiveSubRunControlBoundary, SubRunHandle,
    };
    use astrcode_runtime_config::{DEFAULT_MAX_AGENT_DEPTH, DEFAULT_MAX_CONCURRENT_AGENTS};

    use super::{AgentControl, AgentControlError, LiveSubRunControl, StaticAgentProfileSource};

    fn explore_profile() -> AgentProfile {
        AgentProfile {
            id: "explore".to_string(),
            name: "Explore".to_string(),
            description: "只读探索".to_string(),
            mode: AgentMode::SubAgent,
            system_prompt: None,
            allowed_tools: vec!["readFile".to_string()],
            disallowed_tools: Vec::new(),
            // TODO: 未来可能需要添加 max_steps 和 token_budget
            model_preference: Some("fast".to_string()),
        }
    }

    fn sample_parent_delivery(
        notification_id: &str,
        parent_session_id: &str,
        parent_turn_id: &str,
    ) -> (String, String, ChildSessionNotification) {
        (
            parent_session_id.to_string(),
            parent_turn_id.to_string(),
            ChildSessionNotification {
                notification_id: notification_id.to_string(),
                child_ref: ChildAgentRef {
                    agent_id: format!("agent-{notification_id}"),
                    session_id: parent_session_id.to_string(),
                    sub_run_id: format!("subrun-{notification_id}"),
                    parent_agent_id: None,
                    lineage_kind: ChildSessionLineageKind::Spawn,
                    status: AgentStatus::Completed,
                    open_session_id: format!("child-session-{notification_id}"),
                },
                kind: ChildSessionNotificationKind::Delivered,
                summary: format!("summary-{notification_id}"),
                status: AgentStatus::Completed,
                source_tool_call_id: None,
                final_reply_excerpt: Some(format!("final-{notification_id}")),
            },
        )
    }

    fn sample_parent_delivery_for_child(
        notification_id: &str,
        parent_session_id: &str,
        _parent_turn_id: &str,
        child: &SubRunHandle,
    ) -> ChildSessionNotification {
        ChildSessionNotification {
            notification_id: notification_id.to_string(),
            child_ref: ChildAgentRef {
                agent_id: child.agent_id.clone(),
                session_id: parent_session_id.to_string(),
                sub_run_id: child.sub_run_id.clone(),
                parent_agent_id: child.parent_agent_id.clone(),
                lineage_kind: ChildSessionLineageKind::Spawn,
                status: child.status,
                open_session_id: child
                    .child_session_id
                    .clone()
                    .unwrap_or_else(|| child.session_id.clone()),
            },
            kind: ChildSessionNotificationKind::Delivered,
            summary: format!("summary-{notification_id}"),
            status: child.status,
            source_tool_call_id: None,
            final_reply_excerpt: Some(format!("final-{notification_id}")),
        }
    }

    #[tokio::test]
    async fn spawn_list_and_wait_track_status() {
        let control = AgentControl::new();
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");

        assert_eq!(handle.status, AgentStatus::Pending);
        assert_eq!(control.list().await.len(), 1);

        let agent_id = handle.agent_id.clone();
        let waiter = {
            let control = control.clone();
            tokio::spawn(async move { control.wait(&agent_id).await })
        };
        // 先让 waiter 完成订阅，避免测试依赖调度时序而偶发卡住。
        tokio::task::yield_now().await;

        let running = control
            .mark_running(&handle.agent_id)
            .await
            .expect("agent should exist");
        assert_eq!(running.status, AgentStatus::Running);

        let completed = control
            .mark_completed(&handle.agent_id)
            .await
            .expect("agent should exist");
        assert_eq!(completed.status, AgentStatus::Completed);

        let waited = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("waiter should finish before timeout")
            .expect("waiter should join");
        assert_eq!(
            waited.expect("wait should resolve").status,
            AgentStatus::Completed
        );
    }

    #[tokio::test]
    async fn cancelling_parent_turn_cascades_to_children() {
        // 需要 depth ≥ 2 才能测试 parent → child 嵌套
        let control = AgentControl::with_limits(3, 10, 256);
        let parent = control
            .spawn(
                &explore_profile(),
                "session-parent",
                "turn-root".to_string(),
                None,
            )
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&parent.agent_id).await;

        let child = control
            .spawn(
                &explore_profile(),
                "session-child",
                "turn-root".to_string(),
                Some(parent.agent_id.clone()),
            )
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&child.agent_id).await;

        let cancelled = control.cancel_for_parent_turn("turn-root").await;
        assert_eq!(cancelled.len(), 2);

        let parent_handle = control
            .get(&parent.agent_id)
            .await
            .expect("parent should exist");
        let child_handle = control
            .get(&child.agent_id)
            .await
            .expect("child should exist");
        assert_eq!(parent_handle.status, AgentStatus::Cancelled);
        assert_eq!(child_handle.status, AgentStatus::Cancelled);

        let child_cancel = control
            .cancel_token(&child.agent_id)
            .await
            .expect("child cancel token should exist");
        assert!(child_cancel.is_cancelled());
    }

    #[tokio::test]
    async fn spawn_rejects_unknown_parent_agent() {
        let control = AgentControl::new();

        let error = control
            .spawn(
                &explore_profile(),
                "session-1",
                "turn-1".to_string(),
                Some("missing-parent".to_string()),
            )
            .await
            .expect_err("spawn should reject unknown parent");

        assert_eq!(
            error,
            AgentControlError::ParentAgentNotFound {
                agent_id: "missing-parent".to_string(),
            }
        );
        assert!(control.list().await.is_empty());
    }

    #[tokio::test]
    async fn failed_spawn_does_not_consume_agent_id() {
        let control = AgentControl::new();

        let _ = control
            .spawn(
                &explore_profile(),
                "session-1",
                "turn-1".to_string(),
                Some("missing-parent".to_string()),
            )
            .await
            .expect_err("spawn should reject unknown parent");

        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("first successful spawn should still get the first id");

        assert_eq!(handle.agent_id, "agent-1");
    }

    #[tokio::test]
    async fn cancel_directly_cascades_to_child_tree() {
        // 需要 depth ≥ 3 才能测试 parent → child → grandchild 嵌套
        let control = AgentControl::with_limits(3, 10, 256);
        let parent = control
            .spawn(
                &explore_profile(),
                "session-parent",
                "turn-root".to_string(),
                None,
            )
            .await
            .expect("parent spawn should succeed");
        let child = control
            .spawn(
                &explore_profile(),
                "session-child",
                "turn-root".to_string(),
                Some(parent.agent_id.clone()),
            )
            .await
            .expect("child spawn should succeed");
        let grandchild = control
            .spawn(
                &explore_profile(),
                "session-grandchild",
                "turn-root".to_string(),
                Some(child.agent_id.clone()),
            )
            .await
            .expect("grandchild spawn should succeed");
        let _ = control.mark_running(&parent.agent_id).await;
        let _ = control.mark_running(&child.agent_id).await;
        let _ = control.mark_running(&grandchild.agent_id).await;

        let cancelled = control
            .cancel(&parent.agent_id)
            .await
            .expect("parent cancel should exist");
        assert_eq!(cancelled.status, AgentStatus::Cancelled);

        for agent_id in [&parent.agent_id, &child.agent_id, &grandchild.agent_id] {
            let handle = control
                .get(agent_id)
                .await
                .expect("agent should still exist");
            assert_eq!(handle.status, AgentStatus::Cancelled);
        }
    }

    #[tokio::test]
    async fn mark_failed_transitions_agent_to_final_failed_state() {
        let control = AgentControl::new();
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&handle.agent_id).await;

        let failed = control
            .mark_failed(&handle.agent_id)
            .await
            .expect("agent should exist");
        assert_eq!(failed.status, AgentStatus::Failed);

        let waited = control
            .wait(&handle.agent_id)
            .await
            .expect("failed agent should still be queryable");
        assert_eq!(waited.status, AgentStatus::Failed);
    }

    #[tokio::test]
    async fn gc_prunes_old_finalized_leaf_agents_but_keeps_recent_and_live_nodes() {
        let control =
            AgentControl::with_limits(DEFAULT_MAX_AGENT_DEPTH, DEFAULT_MAX_CONCURRENT_AGENTS, 1);

        let first = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("first spawn should succeed");
        let second = control
            .spawn(&explore_profile(), "session-1", "turn-2".to_string(), None)
            .await
            .expect("second spawn should succeed");
        let live = control
            .spawn(&explore_profile(), "session-1", "turn-3".to_string(), None)
            .await
            .expect("live spawn should succeed");

        let _ = control.mark_completed(&first.agent_id).await;
        let _ = control.mark_failed(&second.agent_id).await;

        let handles = control.list().await;
        assert_eq!(
            handles.len(),
            2,
            "gc should evict the oldest finalized leaf"
        );
        assert!(control.get(&first.agent_id).await.is_none());
        assert_eq!(
            control
                .get(&second.agent_id)
                .await
                .expect("newer finalized agent")
                .status,
            AgentStatus::Failed
        );
        assert_eq!(
            control
                .get(&live.agent_id)
                .await
                .expect("live agent should remain")
                .status,
            AgentStatus::Pending
        );
    }

    #[tokio::test]
    async fn spawn_rejects_agents_that_exceed_max_depth() {
        let control = AgentControl::with_limits(2, 8, usize::MAX);
        let root = control
            .spawn(
                &explore_profile(),
                "session-root",
                "turn-root".to_string(),
                None,
            )
            .await
            .expect("root should fit within depth 1");
        let child = control
            .spawn(
                &explore_profile(),
                "session-child",
                "turn-root".to_string(),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("child should fit within depth 2");
        assert_eq!(root.depth, 1);
        assert_eq!(child.depth, 2);

        let error = control
            .spawn(
                &explore_profile(),
                "session-grandchild",
                "turn-root".to_string(),
                Some(child.agent_id.clone()),
            )
            .await
            .expect_err("grandchild should exceed max depth");
        assert_eq!(
            error,
            AgentControlError::MaxDepthExceeded { current: 3, max: 2 }
        );
    }

    #[tokio::test]
    async fn finalized_agents_release_concurrency_slots() {
        let control = AgentControl::with_limits(8, 2, usize::MAX);
        let first = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("first spawn should succeed");
        let second = control
            .spawn(&explore_profile(), "session-2", "turn-2".to_string(), None)
            .await
            .expect("second spawn should succeed");

        let error = control
            .spawn(&explore_profile(), "session-3", "turn-3".to_string(), None)
            .await
            .expect_err("third active agent should exceed concurrent limit");
        assert_eq!(
            error,
            AgentControlError::MaxConcurrentExceeded { current: 2, max: 2 }
        );

        let _ = control.mark_completed(&first.agent_id).await;
        let third = control
            .spawn(&explore_profile(), "session-3", "turn-3".to_string(), None)
            .await
            .expect("finalizing one agent should release a slot");
        assert_eq!(third.depth, 1);
        assert_eq!(
            control
                .get(&second.agent_id)
                .await
                .expect("second should still exist")
                .status,
            AgentStatus::Pending
        );
    }

    #[tokio::test]
    async fn live_subrun_control_surface_delegates_registry_and_profiles() {
        let control = AgentControl::new();
        let profile = explore_profile();
        let handle = control
            .spawn(&profile, "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let surface = LiveSubRunControl::new(
            control.clone(),
            StaticAgentProfileSource::new(vec![profile.clone()]),
        );

        let loaded = surface
            .get_subrun_handle("session-1", &handle.sub_run_id)
            .await
            .expect("lookup should succeed")
            .expect("handle should exist");
        assert_eq!(loaded.agent_id, handle.agent_id);
        assert_eq!(
            surface
                .list_profiles()
                .await
                .expect("profiles should load")
                .len(),
            1
        );

        surface
            .cancel_subrun("session-1", &handle.sub_run_id)
            .await
            .expect("cancel should succeed");
        assert_eq!(
            control
                .get(&handle.sub_run_id)
                .await
                .expect("handle should remain visible")
                .status,
            AgentStatus::Cancelled
        );
    }

    // ─── T028 协作操作运行时测试 ───────────────────────────

    #[tokio::test]
    async fn targeted_wait_resolves_only_specific_agent_not_siblings() {
        let control = AgentControl::with_limits(3, 10, 256);
        let agent_a = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("agent A spawn should succeed");
        let agent_b = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("agent B spawn should succeed");
        let _ = control.mark_running(&agent_a.agent_id).await;
        let _ = control.mark_running(&agent_b.agent_id).await;

        // 只完成 agent_a，agent_b 仍运行中
        let _ = control.mark_completed(&agent_a.agent_id).await;

        // wait 应该立即返回已终态的 agent_a
        let waited = control
            .wait(&agent_a.agent_id)
            .await
            .expect("wait should resolve");
        assert_eq!(waited.status, AgentStatus::Completed);

        // agent_b 仍然处于 Running 状态，不受影响
        let b_handle = control
            .get(&agent_b.agent_id)
            .await
            .expect("agent B should exist");
        assert_eq!(b_handle.status, AgentStatus::Running);
    }

    #[tokio::test]
    async fn resume_mints_new_execution_for_completed_agent() {
        let control = AgentControl::with_limits(3, 10, 256);
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&handle.agent_id).await;
        let _ = control.mark_completed(&handle.agent_id).await;

        // 恢复已完成的 agent
        let resumed = control
            .resume(&handle.agent_id)
            .await
            .expect("resume should succeed");
        assert_eq!(resumed.status, AgentStatus::Running);
        assert_eq!(resumed.agent_id, handle.agent_id);
        assert_ne!(
            resumed.sub_run_id, handle.sub_run_id,
            "resume should mint a new execution id"
        );

        let historical = control
            .get(&handle.sub_run_id)
            .await
            .expect("historical execution should remain queryable by old sub-run id");
        assert_eq!(historical.status, AgentStatus::Completed);

        // 验证恢复后能再次正常到达终态
        let _ = control.mark_completed(&handle.agent_id).await;
        let final_handle = control
            .get(&handle.agent_id)
            .await
            .expect("agent should exist");
        assert_eq!(final_handle.status, AgentStatus::Completed);
    }

    #[tokio::test]
    async fn resume_rejects_non_final_agent() {
        let control = AgentControl::with_limits(3, 10, 256);
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&handle.agent_id).await;

        // Running 状态的 agent 不能被恢复
        let result = control.resume(&handle.agent_id).await;
        assert!(result.is_none(), "running agent should not be resumable");
    }

    #[tokio::test]
    async fn close_cascades_to_entire_subtree_but_not_siblings() {
        let control = AgentControl::with_limits(4, 10, 256);

        // 构建两棵独立子树
        let tree_a_parent = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("tree A parent spawn should succeed");
        let tree_a_child = control
            .spawn(
                &explore_profile(),
                "session-1",
                "turn-1".to_string(),
                Some(tree_a_parent.agent_id.clone()),
            )
            .await
            .expect("tree A child spawn should succeed");

        let tree_b_parent = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("tree B parent spawn should succeed");
        let _tree_b_child = control
            .spawn(
                &explore_profile(),
                "session-1",
                "turn-1".to_string(),
                Some(tree_b_parent.agent_id.clone()),
            )
            .await
            .expect("tree B child spawn should succeed");

        let _ = control.mark_running(&tree_a_parent.agent_id).await;
        let _ = control.mark_running(&tree_a_child.agent_id).await;
        let _ = control.mark_running(&tree_b_parent.agent_id).await;

        // 关闭 tree A 的根，应级联到 tree A 的 child
        let cancelled = control
            .cancel(&tree_a_parent.agent_id)
            .await
            .expect("cancel should succeed");
        assert_eq!(cancelled.status, AgentStatus::Cancelled);

        // tree A 的 parent 和 child 都被取消
        assert_eq!(
            control
                .get(&tree_a_parent.agent_id)
                .await
                .expect("should exist")
                .status,
            AgentStatus::Cancelled
        );
        assert_eq!(
            control
                .get(&tree_a_child.agent_id)
                .await
                .expect("should exist")
                .status,
            AgentStatus::Cancelled
        );

        // tree B 不受影响
        assert_eq!(
            control
                .get(&tree_b_parent.agent_id)
                .await
                .expect("should exist")
                .status,
            AgentStatus::Running
        );
    }

    #[tokio::test]
    async fn resume_reoccupies_concurrency_slot() {
        let control = AgentControl::with_limits(8, 2, usize::MAX);
        let first = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("first spawn should succeed");
        let _second = control
            .spawn(&explore_profile(), "session-2", "turn-2".to_string(), None)
            .await
            .expect("second spawn should succeed");

        let _ = control.mark_running(&first.agent_id).await;
        let _ = control.mark_completed(&first.agent_id).await;

        // first 完成后释放了槽位，可以创建第三个
        let _third = control
            .spawn(&explore_profile(), "session-3", "turn-3".to_string(), None)
            .await
            .expect("third spawn should succeed after first completed");

        // 恢复 first 会重新占用槽位，此时已有 3 个活跃（first resumed + second + third）
        let _ = control.resume(&first.agent_id).await;

        let error = control
            .spawn(&explore_profile(), "session-4", "turn-4".to_string(), None)
            .await
            .expect_err("should exceed concurrent limit after resume");
        assert_eq!(
            error,
            AgentControlError::MaxConcurrentExceeded { current: 3, max: 2 }
        );
    }

    // ─── 收件箱测试 ──────────────────────────────────────

    fn sample_envelope(id: &str, from: &str, to: &str, message: &str) -> AgentInboxEnvelope {
        AgentInboxEnvelope {
            delivery_id: id.to_string(),
            from_agent_id: from.to_string(),
            to_agent_id: to.to_string(),
            kind: astrcode_core::InboxEnvelopeKind::ParentMessage,
            message: message.to_string(),
            context: None,
            is_final: false,
            summary: None,
            findings: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    #[tokio::test]
    async fn push_and_drain_inbox_enqueues_and_consumes_envelopes() {
        let control = AgentControl::new();
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&handle.agent_id).await;

        // 推送两封信封
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-1", "agent-parent", &handle.agent_id, "请修改"),
            )
            .await
            .expect("push should succeed");
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-2", "agent-parent", &handle.agent_id, "补充说明"),
            )
            .await
            .expect("push should succeed");

        // 排空收件箱
        let envelopes = control
            .drain_inbox(&handle.agent_id)
            .await
            .expect("drain should succeed");
        assert_eq!(envelopes.len(), 2);
        assert_eq!(envelopes[0].delivery_id, "d-1");
        assert_eq!(envelopes[1].delivery_id, "d-2");

        // 二次排空为空
        let empty = control
            .drain_inbox(&handle.agent_id)
            .await
            .expect("drain should succeed");
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn complete_turn_moves_agent_into_idle_with_last_outcome() {
        let control = AgentControl::new();
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let _ = control
            .mark_running(&handle.agent_id)
            .await
            .expect("mark running should succeed");

        let lifecycle = control
            .complete_turn(&handle.agent_id, AgentTurnOutcome::Completed)
            .await
            .expect("complete turn should succeed");
        assert_eq!(lifecycle, AgentLifecycleStatus::Idle);
        assert_eq!(
            control.get_lifecycle(&handle.agent_id).await,
            Some(AgentLifecycleStatus::Idle)
        );
        assert_eq!(
            control.get_turn_outcome(&handle.agent_id).await,
            Some(Some(AgentTurnOutcome::Completed))
        );
        assert_eq!(
            control
                .get(&handle.agent_id)
                .await
                .expect("completed handle should remain queryable")
                .status,
            AgentStatus::Completed
        );
    }

    #[tokio::test]
    async fn push_inbox_deduplication_by_delivery_id() {
        let control = AgentControl::new();
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");

        // 推送相同 delivery_id 的信封两次
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-dup", "agent-parent", &handle.agent_id, "消息"),
            )
            .await
            .expect("push should succeed");
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-dup", "agent-parent", &handle.agent_id, "消息"),
            )
            .await
            .expect("push should succeed");

        // 当前实现不内置去重，由调用方保证幂等；
        // 验证两封信封都入队（调用方负责 dedupe 语义）
        let envelopes = control
            .drain_inbox(&handle.agent_id)
            .await
            .expect("drain should succeed");
        assert_eq!(envelopes.len(), 2);
    }

    #[tokio::test]
    async fn wait_for_inbox_resolves_on_new_envelope() {
        let control = AgentControl::new();
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&handle.agent_id).await;

        let agent_id = handle.agent_id.clone();
        let control_clone = control.clone();
        let waiter = tokio::spawn(async move { control_clone.wait_for_inbox(&agent_id).await });

        // 让 waiter 完成订阅
        tokio::task::yield_now().await;

        // 推送信封唤醒 waiter
        control
            .push_inbox(
                &handle.agent_id,
                sample_envelope("d-wait", "agent-parent", &handle.agent_id, "唤醒"),
            )
            .await
            .expect("push should succeed");

        let result = tokio::time::timeout(Duration::from_secs(3), waiter)
            .await
            .expect("waiter should finish before timeout")
            .expect("waiter should join");
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn terminate_subtree_clears_pending_inbox_messages() {
        let control = AgentControl::with_limits(4, 16, 256);
        let parent = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("parent spawn should succeed");
        let child = control
            .spawn(
                &explore_profile(),
                "session-1",
                "turn-1".to_string(),
                Some(parent.agent_id.clone()),
            )
            .await
            .expect("child spawn should succeed");
        let _ = control.mark_running(&parent.agent_id).await;
        let _ = control.mark_running(&child.agent_id).await;

        control
            .push_inbox(
                &child.agent_id,
                sample_envelope("d-close", "agent-parent", &child.agent_id, "终止前排队消息"),
            )
            .await
            .expect("push should succeed");

        control
            .terminate_subtree(&parent.agent_id)
            .await
            .expect("terminate subtree should succeed");

        let child_inbox = control
            .drain_inbox(&child.agent_id)
            .await
            .expect("drain should succeed after close");
        assert!(child_inbox.is_empty());
        assert_eq!(
            control.get_lifecycle(&child.agent_id).await,
            Some(AgentLifecycleStatus::Terminated)
        );
    }

    #[tokio::test]
    async fn terminate_subtree_discards_pending_parent_deliveries_for_closed_branch() {
        let control = AgentControl::with_limits(4, 16, 256);
        let root = control
            .spawn(
                &explore_profile(),
                "session-parent",
                "turn-root".to_string(),
                None,
            )
            .await
            .expect("root spawn should succeed");
        let child = control
            .spawn(
                &explore_profile(),
                "session-child-a",
                "turn-root".to_string(),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("child spawn should succeed");
        let sibling = control
            .spawn(
                &explore_profile(),
                "session-child-b",
                "turn-root".to_string(),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("sibling spawn should succeed");

        let session_id = "session-parent".to_string();
        let turn_id = "turn-root".to_string();
        assert!(
            control
                .enqueue_parent_delivery(
                    session_id.clone(),
                    turn_id.clone(),
                    sample_parent_delivery_for_child(
                        "closed-branch",
                        &session_id,
                        &turn_id,
                        &child,
                    )
                )
                .await
        );
        assert!(
            control
                .enqueue_parent_delivery(
                    session_id.clone(),
                    turn_id.clone(),
                    sample_parent_delivery_for_child(
                        "live-branch",
                        &session_id,
                        &turn_id,
                        &sibling,
                    )
                )
                .await
        );
        assert_eq!(control.pending_parent_delivery_count(&session_id).await, 2);

        control
            .terminate_subtree(&child.agent_id)
            .await
            .expect("terminate should succeed");

        assert_eq!(control.pending_parent_delivery_count(&session_id).await, 1);
        let remaining = control
            .checkout_parent_delivery(&session_id)
            .await
            .expect("sibling delivery should remain queued");
        assert_eq!(remaining.notification.child_ref.agent_id, sibling.agent_id);
    }

    #[tokio::test]
    async fn wait_for_inbox_returns_immediately_for_final_agent() {
        let control = AgentControl::new();
        let handle = control
            .spawn(&explore_profile(), "session-1", "turn-1".to_string(), None)
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&handle.agent_id).await;
        let _ = control.mark_completed(&handle.agent_id).await;

        let result = control
            .wait_for_inbox(&handle.agent_id)
            .await
            .expect("should resolve immediately");
        assert_eq!(result.status, AgentStatus::Completed);
    }

    #[tokio::test]
    async fn push_inbox_returns_none_for_nonexistent_agent() {
        let control = AgentControl::new();
        let result = control
            .push_inbox(
                "missing-agent",
                sample_envelope("d-1", "agent-parent", "missing-agent", "消息"),
            )
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn parent_delivery_queue_deduplicates_and_preserves_fifo_order() {
        let control = AgentControl::new();
        let (session_id, turn_id, first) =
            sample_parent_delivery("delivery-1", "session-parent", "turn-parent");
        let (_, _, duplicate) =
            sample_parent_delivery("delivery-1", "session-parent", "turn-parent");
        let (_, _, second) = sample_parent_delivery("delivery-2", "session-parent", "turn-parent");

        assert!(
            control
                .enqueue_parent_delivery(session_id.clone(), turn_id.clone(), first)
                .await
        );
        assert!(
            !control
                .enqueue_parent_delivery(session_id.clone(), turn_id.clone(), duplicate)
                .await
        );
        assert!(
            control
                .enqueue_parent_delivery(session_id.clone(), turn_id, second)
                .await
        );

        let first_checked_out = control
            .checkout_parent_delivery(&session_id)
            .await
            .expect("first queued delivery should be available");
        assert_eq!(first_checked_out.delivery_id, "delivery-1");
        assert!(
            control
                .consume_parent_delivery(&session_id, &first_checked_out.delivery_id)
                .await
        );

        let second_checked_out = control
            .checkout_parent_delivery(&session_id)
            .await
            .expect("second queued delivery should be available");
        assert_eq!(second_checked_out.delivery_id, "delivery-2");
        assert_eq!(control.pending_parent_delivery_count(&session_id).await, 1);
    }

    #[tokio::test]
    async fn parent_delivery_queue_can_requeue_busy_head_without_losing_it() {
        let control = AgentControl::new();
        let (session_id, turn_id, delivery) =
            sample_parent_delivery("delivery-busy", "session-parent", "turn-parent");

        assert!(
            control
                .enqueue_parent_delivery(session_id.clone(), turn_id, delivery)
                .await
        );

        let checked_out = control
            .checkout_parent_delivery(&session_id)
            .await
            .expect("delivery should be checked out");
        assert!(
            control
                .checkout_parent_delivery(&session_id)
                .await
                .is_none(),
            "waking delivery should block duplicate checkout"
        );

        assert!(
            control
                .requeue_parent_delivery(&session_id, &checked_out.delivery_id)
                .await
        );

        let retried = control
            .checkout_parent_delivery(&session_id)
            .await
            .expect("requeued delivery should become available again");
        assert_eq!(retried.delivery_id, checked_out.delivery_id);
        assert!(
            control
                .consume_parent_delivery(&session_id, &retried.delivery_id)
                .await
        );
        assert_eq!(control.pending_parent_delivery_count(&session_id).await, 0);
    }

    #[tokio::test]
    async fn parent_delivery_batch_checkout_uses_turn_start_snapshot_for_same_parent_agent() {
        let control = AgentControl::new();
        let session_id = "session-parent".to_string();
        let turn_id = "turn-parent".to_string();
        let make_delivery =
            |delivery_id: &str, child_id: &str, parent_agent_id: &str| ChildSessionNotification {
                notification_id: delivery_id.to_string(),
                child_ref: ChildAgentRef {
                    agent_id: child_id.to_string(),
                    session_id: session_id.clone(),
                    sub_run_id: format!("subrun-{delivery_id}"),
                    parent_agent_id: Some(parent_agent_id.to_string()),
                    lineage_kind: ChildSessionLineageKind::Spawn,
                    status: AgentStatus::Completed,
                    open_session_id: format!("child-session-{delivery_id}"),
                },
                kind: ChildSessionNotificationKind::Delivered,
                summary: format!("summary-{delivery_id}"),
                status: AgentStatus::Completed,
                source_tool_call_id: None,
                final_reply_excerpt: Some(format!("final-{delivery_id}")),
            };

        assert!(
            control
                .enqueue_parent_delivery(
                    session_id.clone(),
                    turn_id.clone(),
                    make_delivery("delivery-1", "agent-child-1", "agent-parent-a"),
                )
                .await
        );
        assert!(
            control
                .enqueue_parent_delivery(
                    session_id.clone(),
                    turn_id.clone(),
                    make_delivery("delivery-2", "agent-child-2", "agent-parent-a"),
                )
                .await
        );
        assert!(
            control
                .enqueue_parent_delivery(
                    session_id.clone(),
                    turn_id,
                    make_delivery("delivery-3", "agent-child-3", "agent-parent-b"),
                )
                .await
        );

        let first_batch = control
            .checkout_parent_delivery_batch(&session_id)
            .await
            .expect("same parent-agent head deliveries should form a batch");
        assert_eq!(
            first_batch
                .iter()
                .map(|delivery| delivery.delivery_id.as_str())
                .collect::<Vec<_>>(),
            vec!["delivery-1", "delivery-2"]
        );
        assert!(
            control
                .checkout_parent_delivery_batch(&session_id)
                .await
                .is_none(),
            "head batch is already waking; next batch must wait for consume/requeue"
        );
        assert!(
            control
                .consume_parent_delivery_batch(
                    &session_id,
                    &first_batch
                        .iter()
                        .map(|delivery| delivery.delivery_id.clone())
                        .collect::<Vec<_>>(),
                )
                .await
        );

        let second_batch = control
            .checkout_parent_delivery_batch(&session_id)
            .await
            .expect("next parent-agent group should become the next batch");
        assert_eq!(second_batch.len(), 1);
        assert_eq!(second_batch[0].delivery_id, "delivery-3");
    }

    #[tokio::test]
    async fn parent_delivery_batch_requeue_restores_started_snapshot_for_retry() {
        let control = AgentControl::new();
        let session_id = "session-parent".to_string();
        let turn_id = "turn-parent".to_string();
        let make_delivery =
            |delivery_id: &str, child_id: &str, parent_agent_id: &str| ChildSessionNotification {
                notification_id: delivery_id.to_string(),
                child_ref: ChildAgentRef {
                    agent_id: child_id.to_string(),
                    session_id: session_id.clone(),
                    sub_run_id: format!("subrun-{delivery_id}"),
                    parent_agent_id: Some(parent_agent_id.to_string()),
                    lineage_kind: ChildSessionLineageKind::Spawn,
                    status: AgentStatus::Completed,
                    open_session_id: format!("child-session-{delivery_id}"),
                },
                kind: ChildSessionNotificationKind::Delivered,
                summary: format!("summary-{delivery_id}"),
                status: AgentStatus::Completed,
                source_tool_call_id: None,
                final_reply_excerpt: Some(format!("final-{delivery_id}")),
            };

        assert!(
            control
                .enqueue_parent_delivery(
                    session_id.clone(),
                    turn_id.clone(),
                    make_delivery("delivery-1", "agent-child-1", "agent-parent-a"),
                )
                .await
        );
        assert!(
            control
                .enqueue_parent_delivery(
                    session_id.clone(),
                    turn_id,
                    make_delivery("delivery-2", "agent-child-2", "agent-parent-a"),
                )
                .await
        );

        let started_batch = control
            .checkout_parent_delivery_batch(&session_id)
            .await
            .expect("queued deliveries should form a started batch");
        let delivery_ids = started_batch
            .iter()
            .map(|delivery| delivery.delivery_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            delivery_ids,
            vec!["delivery-1".to_string(), "delivery-2".to_string()]
        );

        assert_eq!(
            control
                .requeue_parent_delivery_batch(&session_id, &delivery_ids)
                .await,
            2
        );

        let replayed_batch = control
            .checkout_parent_delivery_batch(&session_id)
            .await
            .expect("requeued started batch should become available again");
        assert_eq!(
            replayed_batch
                .iter()
                .map(|delivery| delivery.delivery_id.as_str())
                .collect::<Vec<_>>(),
            vec!["delivery-1", "delivery-2"]
        );
    }

    // ─── T035 层级协作回归测试 ──────────────────────────────

    /// 验证级联关闭是 leaf-first 语义：
    /// 三层链 root → middle → leaf，关闭 middle 时，
    /// leaf 先被取消（子树从叶子向上传播），root 不受影响。
    #[tokio::test]
    async fn leaf_first_cascade_cancels_deepest_child_before_parent() {
        let control = AgentControl::with_limits(4, 10, 256);

        let root = control
            .spawn(
                &explore_profile(),
                "session-root",
                "turn-1".to_string(),
                None,
            )
            .await
            .expect("root spawn should succeed");
        let middle = control
            .spawn(
                &explore_profile(),
                "session-middle",
                "turn-1".to_string(),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle spawn should succeed");
        let leaf = control
            .spawn(
                &explore_profile(),
                "session-leaf",
                "turn-1".to_string(),
                Some(middle.agent_id.clone()),
            )
            .await
            .expect("leaf spawn should succeed");
        let _ = control.mark_running(&root.agent_id).await;
        let _ = control.mark_running(&middle.agent_id).await;
        let _ = control.mark_running(&leaf.agent_id).await;

        // 关闭 middle，应级联到 leaf，但不影响 root
        let cancelled = control
            .cancel(&middle.agent_id)
            .await
            .expect("cancel should succeed");
        assert_eq!(cancelled.status, AgentStatus::Cancelled);

        // middle 和 leaf 都被取消
        assert_eq!(
            control
                .get(&middle.agent_id)
                .await
                .expect("middle should exist")
                .status,
            AgentStatus::Cancelled
        );
        assert_eq!(
            control
                .get(&leaf.agent_id)
                .await
                .expect("leaf should exist")
                .status,
            AgentStatus::Cancelled
        );

        // root 不受影响
        assert_eq!(
            control
                .get(&root.agent_id)
                .await
                .expect("root should exist")
                .status,
            AgentStatus::Running
        );
    }

    /// 验证子树隔离：关闭一个分支的中间节点不会影响兄弟分支。
    /// root → middle_a → leaf_a
    /// root → middle_b → leaf_b
    /// 关闭 middle_a 只影响 middle_a + leaf_a，middle_b + leaf_b 不受影响。
    #[tokio::test]
    async fn subtree_isolation_closing_one_branch_does_not_affect_sibling_branch() {
        let control = AgentControl::with_limits(4, 10, 256);

        let root = control
            .spawn(
                &explore_profile(),
                "session-root",
                "turn-1".to_string(),
                None,
            )
            .await
            .expect("root spawn should succeed");
        let middle_a = control
            .spawn(
                &explore_profile(),
                "session-middle-a",
                "turn-1".to_string(),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle_a spawn should succeed");
        let leaf_a = control
            .spawn(
                &explore_profile(),
                "session-leaf-a",
                "turn-1".to_string(),
                Some(middle_a.agent_id.clone()),
            )
            .await
            .expect("leaf_a spawn should succeed");
        let middle_b = control
            .spawn(
                &explore_profile(),
                "session-middle-b",
                "turn-1".to_string(),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle_b spawn should succeed");
        let leaf_b = control
            .spawn(
                &explore_profile(),
                "session-leaf-b",
                "turn-1".to_string(),
                Some(middle_b.agent_id.clone()),
            )
            .await
            .expect("leaf_b spawn should succeed");

        let _ = control.mark_running(&root.agent_id).await;
        let _ = control.mark_running(&middle_a.agent_id).await;
        let _ = control.mark_running(&leaf_a.agent_id).await;
        let _ = control.mark_running(&middle_b.agent_id).await;
        let _ = control.mark_running(&leaf_b.agent_id).await;

        // 关闭 middle_a 分支
        let _ = control
            .cancel(&middle_a.agent_id)
            .await
            .expect("cancel should succeed");

        // branch A 全部被取消
        assert_eq!(
            control
                .get(&middle_a.agent_id)
                .await
                .expect("middle_a should exist")
                .status,
            AgentStatus::Cancelled
        );
        assert_eq!(
            control
                .get(&leaf_a.agent_id)
                .await
                .expect("leaf_a should exist")
                .status,
            AgentStatus::Cancelled
        );

        // branch B 完全不受影响
        assert_eq!(
            control
                .get(&middle_b.agent_id)
                .await
                .expect("middle_b should exist")
                .status,
            AgentStatus::Running
        );
        assert_eq!(
            control
                .get(&leaf_b.agent_id)
                .await
                .expect("leaf_b should exist")
                .status,
            AgentStatus::Running
        );

        // root 也不受影响
        assert_eq!(
            control
                .get(&root.agent_id)
                .await
                .expect("root should exist")
                .status,
            AgentStatus::Running
        );
    }

    /// 验证 deliverToParent 只投递给直接父 agent，不越级投递到祖父 agent。
    /// root → middle → leaf
    /// leaf 通过 deliverToParent 只能投递到 middle 的 inbox，
    /// root 的 inbox 不应收到 leaf 的投递。
    #[tokio::test]
    async fn deliver_to_parent_only_reaches_direct_parent_not_grandparent() {
        let control = AgentControl::with_limits(4, 10, 256);

        let root = control
            .spawn(
                &explore_profile(),
                "session-root",
                "turn-1".to_string(),
                None,
            )
            .await
            .expect("root spawn should succeed");
        let middle = control
            .spawn(
                &explore_profile(),
                "session-middle",
                "turn-1".to_string(),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle spawn should succeed");
        let leaf = control
            .spawn(
                &explore_profile(),
                "session-leaf",
                "turn-1".to_string(),
                Some(middle.agent_id.clone()),
            )
            .await
            .expect("leaf spawn should succeed");
        let _ = control.mark_running(&root.agent_id).await;
        let _ = control.mark_running(&middle.agent_id).await;
        let _ = control.mark_running(&leaf.agent_id).await;

        // leaf 向直接父 (middle) 投递
        let leaf_delivery = AgentInboxEnvelope {
            delivery_id: "delivery-leaf-to-middle".to_string(),
            from_agent_id: leaf.agent_id.clone(),
            to_agent_id: middle.agent_id.clone(),
            kind: astrcode_core::InboxEnvelopeKind::ChildDelivery,
            message: "leaf 的结果".to_string(),
            context: None,
            is_final: true,
            summary: Some("leaf 完成了任务".to_string()),
            findings: vec!["发现1".to_string()],
            artifacts: Vec::new(),
        };

        control
            .push_inbox(&middle.agent_id, leaf_delivery)
            .await
            .expect("push to middle should succeed");

        // middle 的 inbox 应该有 leaf 的投递
        let middle_inbox = control
            .drain_inbox(&middle.agent_id)
            .await
            .expect("drain middle inbox should succeed");
        assert_eq!(middle_inbox.len(), 1);
        assert_eq!(middle_inbox[0].from_agent_id, leaf.agent_id);
        assert_eq!(
            middle_inbox[0].kind,
            astrcode_core::InboxEnvelopeKind::ChildDelivery
        );
        assert!(middle_inbox[0].is_final);

        // root 的 inbox 应该为空（leaf 不能越级投递）
        let root_inbox = control
            .drain_inbox(&root.agent_id)
            .await
            .expect("drain root inbox should succeed");
        assert!(
            root_inbox.is_empty(),
            "leaf delivery should not reach grandparent inbox"
        );
    }
}
