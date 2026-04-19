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

mod delivery_queue;
mod state;
mod tree_ops;

use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentProfile, AgentTurnOutcome, AstrError,
    CancelToken, ChildSessionLineageKind, LiveSubRunControlBoundary,
    ResolvedExecutionLimitsSnapshot, SessionId, SpawnAgentParams, SubRunHandle, SubRunResult,
    SubRunStorageMode, ToolContext,
};
use async_trait::async_trait;
use delivery_queue::{
    checkout_parent_delivery_batch_locked, checkout_parent_delivery_locked,
    consume_parent_delivery_batch_locked, consume_parent_delivery_locked,
    enqueue_parent_delivery_locked, pending_parent_delivery_count_locked,
    requeue_parent_delivery_batch_locked, requeue_parent_delivery_locked,
};
use state::{AgentEntry, AgentRegistryState, resolve_entry_key};
use thiserror::Error;
use tokio::sync::{RwLock, watch};
use tree_ops::{
    cancel_tree, cancel_tree_collect, discard_parent_deliveries_locked,
    prune_finalized_agents_locked, terminate_tree_collect,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingParentDelivery {
    pub delivery_id: String,
    pub parent_session_id: String,
    pub parent_turn_id: String,
    pub queued_at_ms: i64,
    pub notification: astrcode_core::ChildSessionNotification,
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
    inbox_capacity: usize,
    parent_delivery_capacity: usize,
    state: Arc<RwLock<AgentRegistryState>>,
}

/// Agent 控制平面的显式限额配置。
///
/// kernel 不再读取 runtime-config；由组合根在构造时把策略值显式注入。
#[derive(Debug, Clone, Copy)]
pub struct AgentControlLimits {
    pub max_depth: usize,
    pub max_concurrent: usize,
    pub finalized_retain_limit: usize,
    pub inbox_capacity: usize,
    pub parent_delivery_capacity: usize,
}

impl Default for AgentControlLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_concurrent: 32,
            finalized_retain_limit: 256,
            inbox_capacity: 1024,
            parent_delivery_capacity: 1024,
        }
    }
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
        _session_id: &SessionId,
        sub_run_id: &str,
    ) -> std::result::Result<Option<SubRunHandle>, AstrError> {
        Ok(self.control.get(sub_run_id).await)
    }

    async fn cancel_subrun(
        &self,
        _session_id: &SessionId,
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
        Self::from_limits(AgentControlLimits::default())
    }
}

impl AgentControl {
    /// 用显式限额构建 AgentControl。
    pub fn from_limits(limits: AgentControlLimits) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            next_finalized_seq: Arc::new(AtomicU64::new(0)),
            max_depth: limits.max_depth,
            max_concurrent: limits.max_concurrent,
            finalized_retain_limit: limits.finalized_retain_limit,
            inbox_capacity: limits.inbox_capacity,
            parent_delivery_capacity: limits.parent_delivery_capacity,
            state: Arc::new(RwLock::new(AgentRegistryState::default())),
        }
    }

    pub fn new() -> Self {
        Self::from_limits(AgentControlLimits::default())
    }

    #[cfg(test)]
    fn with_limits(max_depth: usize, max_concurrent: usize, finalized_retain_limit: usize) -> Self {
        Self {
            max_depth,
            max_concurrent,
            finalized_retain_limit,
            inbox_capacity: 1024,
            parent_delivery_capacity: 1024,
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
            SubRunStorageMode::IndependentSession,
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
        let parent_sub_run_id = parent_agent_id
            .as_ref()
            .and_then(|parent_agent_id| state.agent_index.get(parent_agent_id))
            .cloned();
        let handle = SubRunHandle {
            sub_run_id: sub_run_id.clone().into(),
            agent_id: agent_id.clone().into(),
            session_id: session_id.into(),
            child_session_id: child_session_id.map(Into::into),
            depth,
            parent_turn_id: parent_turn_id.into(),
            parent_agent_id: parent_agent_id.clone().map(Into::into),
            parent_sub_run_id: parent_sub_run_id.map(Into::into),
            lineage_kind: ChildSessionLineageKind::Spawn,
            agent_profile: profile.id.clone(),
            storage_mode,
            lifecycle: AgentLifecycleStatus::Pending,
            last_turn_outcome: None,
            resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
            delegation: None,
        };
        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(handle.lifecycle);
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
            sub_run_id: sub_run_id.clone().into(),
            agent_id: agent_id.clone().into(),
            session_id: session_id.into(),
            child_session_id: None,
            depth: 0,
            parent_turn_id: String::new().into(),
            parent_agent_id: None,
            parent_sub_run_id: None,
            lineage_kind: ChildSessionLineageKind::Spawn,
            agent_profile: profile_id,
            storage_mode: SubRunStorageMode::IndependentSession,
            lifecycle: AgentLifecycleStatus::Running,
            last_turn_outcome: None,
            resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
            delegation: None,
        };
        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(handle.lifecycle);
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
        entry.handle.lifecycle = new_status;
        entry.status_tx.send_replace(new_status);
        Some(())
    }

    /// 更新 agent 的最近一轮执行结果。
    ///
    /// 在 turn 完成（无论是正常完成还是失败）时调用，
    /// 同时将 lifecycle 从 Running 推进到 Idle。
    ///
    /// 四工具模型下 Idle 表示"不在执行但仍存活"，
    /// 并发槽位在此时释放，后续 resume 会重新占用。
    pub async fn complete_turn(
        &self,
        id: &str,
        outcome: AgentTurnOutcome,
    ) -> Option<AgentLifecycleStatus> {
        let next_seq = self.next_finalized_seq.fetch_add(1, Ordering::SeqCst);
        let retain_limit = self.finalized_retain_limit;
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let was_active = {
            let entry = state.entries.get_mut(&key)?;
            let was_active = entry.handle.lifecycle.occupies_slot();
            entry.last_turn_outcome = Some(outcome);
            entry.lifecycle_status = AgentLifecycleStatus::Idle;
            entry.handle.lifecycle = AgentLifecycleStatus::Idle;
            entry.handle.last_turn_outcome = Some(outcome);
            entry.finalized_seq = Some(next_seq);
            entry.status_tx.send_replace(AgentLifecycleStatus::Idle);
            was_active
        };
        if was_active {
            state.active_count = state.active_count.saturating_sub(1);
        }
        prune_finalized_agents_locked(&mut state, retain_limit);
        Some(AgentLifecycleStatus::Idle)
    }

    /// 更新 agent 当前执行实例的 resolved limits 快照。
    pub async fn set_resolved_limits(
        &self,
        id: &str,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.handle.resolved_limits = resolved_limits;
        Some(())
    }

    /// 更新 agent 当前执行实例的 delegation 元数据。
    pub async fn set_delegation(
        &self,
        id: &str,
        delegation: Option<astrcode_core::DelegationMetadata>,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        entry.handle.delegation = delegation;
        Some(())
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

    /// 根据 session_id 查找该 session 的根 agent（depth=0）。
    ///
    /// 为什么需要：`submit_prompt` 路径不经过 `execute_root_agent`，无法直接获得根 agent ID，
    /// 但四工具模型要求 ToolContext 中的 agent_id 正确设置，以便子 agent 建立父子关系。
    pub async fn find_root_agent_for_session(&self, session_id: &str) -> Option<SubRunHandle> {
        let state = self.state.read().await;
        state
            .entries
            .values()
            .find(|entry| entry.handle.depth == 0 && entry.handle.session_id.as_str() == session_id)
            .map(|entry| entry.handle.clone())
    }

    /// 获取某个 Agent 的取消令牌，供真正的执行器复用。
    pub async fn cancel_token(&self, sub_run_or_agent_id: &str) -> Option<CancelToken> {
        let state = self.state.read().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?;
        state.entries.get(key).map(|entry| entry.cancel.clone())
    }

    /// 为已终态的 Agent 创建新的执行实例。
    ///
    /// 只有 Completed/Failed/Cancelled 状态的 Agent 可以被恢复。
    /// 恢复不会篡改旧执行实例，而是为同一个 agent mint 一个新的 `sub_run_id`，
    /// 这样 child session 可以沿用稳定身份，同时把新的执行实例显式暴露出来。
    pub async fn resume(
        &self,
        sub_run_or_agent_id: &str,
        parent_turn_id: impl Into<String>,
    ) -> Option<SubRunHandle> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();

        // 先检查状态是否可恢复：只有 lifecycle 不占槽（Idle/Terminated）才可恢复
        if state
            .entries
            .get(&key)
            .is_none_or(|entry| entry.handle.lifecycle.occupies_slot())
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
        new_handle.sub_run_id = new_sub_run_id.clone().into();
        new_handle.parent_turn_id = parent_turn_id.into().into();
        new_handle.lineage_kind = ChildSessionLineageKind::Resume;
        new_handle.lifecycle = AgentLifecycleStatus::Running;
        new_handle.last_turn_outcome = None;

        let cancel = CancelToken::new();
        let (status_tx, _status_rx) = watch::channel(new_handle.lifecycle);
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
                lifecycle_status: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
            },
        );
        state
            .agent_index
            .insert(new_handle.agent_id.to_string(), new_sub_run_id.clone());

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
        // 只取 parent_turn 的直接子树根，排除嵌套子 agent。
        // 如果 agent 的 parent 也在同一个 turn 下，它是孙子节点，
        // 会被祖父级 cancel_tree 级联处理，此处不重复取消。
        let mut roots = state
            .entries
            .values()
            .filter(|entry| entry.handle.parent_turn_id.as_str() == parent_turn_id)
            .filter(|entry| {
                !entry
                    .parent_agent_id
                    .as_ref()
                    .is_some_and(|parent_agent_id| {
                        state
                            .agent_index
                            .get(parent_agent_id)
                            .and_then(|parent_sub_run_id| state.entries.get(parent_sub_run_id))
                            .is_some_and(|parent| {
                                parent.handle.parent_turn_id.as_str() == parent_turn_id
                            })
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
            // 等到 agent 不再占槽（turn 完成或已终止）
            if !current.occupies_slot() {
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
    /// 若收件箱已满（超过 `inbox_capacity`）则返回 None。
    /// 推送后会递增收件箱版本号，唤醒正在 wait_for_inbox 的调用方。
    pub async fn push_inbox(
        &self,
        sub_run_or_agent_id: &str,
        envelope: AgentInboxEnvelope,
    ) -> Option<()> {
        let mut state = self.state.write().await;
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        let entry = state.entries.get_mut(&key)?;
        if entry.inbox.len() >= self.inbox_capacity {
            log::warn!(
                "inbox 已满 ({}/{}), 丢弃来自 {} 的信封 {}",
                entry.inbox.len(),
                self.inbox_capacity,
                envelope.from_agent_id,
                envelope.delivery_id
            );
            return None;
        }
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
            // 单次读锁内同时获取 handle 和 inbox 状态，避免两次独立读锁竞争
            let (handle, inbox_non_empty) = {
                let state = self.state.read().await;
                let key = resolve_entry_key(&state, sub_run_or_agent_id)?;
                let entry = state.entries.get(key)?;
                let handle = entry.handle.clone();
                let inbox_non_empty = !entry.inbox.is_empty();
                (handle, inbox_non_empty)
            };
            // 如果 agent 已终态（Terminated），直接返回当前 handle
            if handle.lifecycle.is_final() {
                return Some(handle);
            }
            // 如果收件箱非空，返回当前 handle
            if inbox_non_empty {
                return Some(handle);
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
    ///
    /// 若队列已满（超过 `parent_delivery_capacity`）则返回 false。
    pub async fn enqueue_parent_delivery(
        &self,
        parent_session_id: impl Into<String>,
        parent_turn_id: impl Into<String>,
        notification: astrcode_core::ChildSessionNotification,
    ) -> bool {
        let mut state = self.state.write().await;
        enqueue_parent_delivery_locked(
            &mut state,
            self.parent_delivery_capacity,
            parent_session_id.into(),
            parent_turn_id.into(),
            notification,
        )
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
        checkout_parent_delivery_locked(&mut state, parent_session_id)
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
        checkout_parent_delivery_batch_locked(&mut state, parent_session_id)
    }

    /// 将正在唤醒中的交付标记回 `Queued`，用于父会话繁忙或启动失败后的重试。
    pub async fn requeue_parent_delivery(
        &self,
        parent_session_id: &str,
        delivery_id: &str,
    ) -> bool {
        let mut state = self.state.write().await;
        requeue_parent_delivery_locked(&mut state, parent_session_id, delivery_id)
    }

    /// 将一批正在唤醒中的交付重新标记为 `Queued`。
    pub async fn requeue_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> usize {
        let mut state = self.state.write().await;
        requeue_parent_delivery_batch_locked(&mut state, parent_session_id, delivery_ids)
    }

    /// 确认最前面的交付已经被父 turn 消费，并将其从缓冲中移除。
    pub async fn consume_parent_delivery(
        &self,
        parent_session_id: &str,
        delivery_id: &str,
    ) -> bool {
        let mut state = self.state.write().await;
        consume_parent_delivery_locked(&mut state, parent_session_id, delivery_id)
    }

    /// 确认一整个交付批次已经被父 turn 消费，并按 FIFO 从队头移除。
    pub async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool {
        let mut state = self.state.write().await;
        consume_parent_delivery_batch_locked(&mut state, parent_session_id, delivery_ids)
    }

    pub async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        let state = self.state.read().await;
        pending_parent_delivery_count_locked(&state, parent_session_id)
    }

    /// 终止指定 agent 及其整棵子树（四工具模型 close 语义）。
    ///
    /// 四工具模型要求 `close` 后 agent
    /// 进入 `Terminated` 生命周期，且后续 `send` 被拒绝。
    ///
    /// 终止过程中：
    /// 1. 对每个节点设置 lifecycle = Terminated
    /// 2. 触发 cancel token 以中断正在运行的 turn
    /// 3. 级联到所有后代
    pub async fn terminate_subtree(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.terminate_subtree_and_collect_handles(sub_run_or_agent_id)
            .await
            .and_then(|mut handles| handles.drain(..).next())
    }

    pub(crate) async fn terminate_subtree_and_collect_handles(
        &self,
        sub_run_or_agent_id: &str,
    ) -> Option<Vec<SubRunHandle>> {
        let mut state = self.state.write().await;
        let mut visited = HashSet::new();
        let mut terminated = Vec::new();
        let key = resolve_entry_key(&state, sub_run_or_agent_id)?.to_string();
        terminate_tree_collect(
            &mut state,
            &key,
            &mut visited,
            &mut terminated,
            &self.next_finalized_seq,
        )?;
        let terminated_agent_ids = terminated
            .iter()
            .map(|handle| handle.agent_id.clone())
            .collect::<HashSet<_>>();
        discard_parent_deliveries_locked(&mut state, &terminated_agent_ids);
        prune_finalized_agents_locked(&mut state, self.finalized_retain_limit);
        Some(terminated)
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
    /// 用于 send 验证直接父路由。
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

#[cfg(test)]
mod tests;
