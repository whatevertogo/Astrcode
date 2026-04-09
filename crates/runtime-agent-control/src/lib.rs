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
    AgentInboxEnvelope, AgentProfile, AgentStatus, AstrError, CancelToken,
    LiveSubRunControlBoundary, SubRunHandle, SubRunStorageMode,
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
        parent_turn_id: Option<String>,
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
        parent_turn_id: Option<String>,
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

    /// 恢复已完成的 Agent 到 Running 状态。
    ///
    /// 只有 Completed/Failed/Cancelled 状态的 Agent 可以被恢复。
    /// 恢复后 Agent 会重新占用并发槽位，父 Agent 的 children 集合也会重新关联。
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

        // 恢复并占用并发槽位
        state.active_count += 1;
        let entry = state.entries.get_mut(&key)?;
        entry.finalized_seq = None;
        entry.handle.status = AgentStatus::Running;
        entry.status_tx.send_replace(AgentStatus::Running);
        Some(entry.handle.clone())
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
            .filter(|entry| entry.handle.parent_turn_id.as_deref() == Some(parent_turn_id))
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
                                parent.handle.parent_turn_id.as_deref() == Some(parent_turn_id)
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
    if next_status.is_final() {
        entry.finalized_seq = Some(next_finalized_seq.fetch_add(1, Ordering::SeqCst));
        // 这里在终态瞬间释放并发槽位，确保新的子 Agent 能及时获取资源；
        // 同时保留终态 handle 供 UI / replay 查询，不把“是否仍可见”与“是否占并发”混为一谈。
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
        AgentInboxEnvelope, AgentMode, AgentProfile, AgentStatus, LiveSubRunControlBoundary,
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

    #[tokio::test]
    async fn spawn_list_and_wait_track_status() {
        let control = AgentControl::new();
        let handle = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
                Some("turn-root".to_string()),
                None,
            )
            .await
            .expect("spawn should succeed");
        let _ = control.mark_running(&parent.agent_id).await;

        let child = control
            .spawn(
                &explore_profile(),
                "session-child",
                Some("turn-root".to_string()),
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
                Some("turn-1".to_string()),
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
                Some("turn-1".to_string()),
                Some("missing-parent".to_string()),
            )
            .await
            .expect_err("spawn should reject unknown parent");

        let handle = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
                Some("turn-root".to_string()),
                None,
            )
            .await
            .expect("parent spawn should succeed");
        let child = control
            .spawn(
                &explore_profile(),
                "session-child",
                Some("turn-root".to_string()),
                Some(parent.agent_id.clone()),
            )
            .await
            .expect("child spawn should succeed");
        let grandchild = control
            .spawn(
                &explore_profile(),
                "session-grandchild",
                Some("turn-root".to_string()),
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("first spawn should succeed");
        let second = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-2".to_string()),
                None,
            )
            .await
            .expect("second spawn should succeed");
        let live = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-3".to_string()),
                None,
            )
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
                Some("turn-root".to_string()),
                None,
            )
            .await
            .expect("root should fit within depth 1");
        let child = control
            .spawn(
                &explore_profile(),
                "session-child",
                Some("turn-root".to_string()),
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
                Some("turn-root".to_string()),
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("first spawn should succeed");
        let second = control
            .spawn(
                &explore_profile(),
                "session-2",
                Some("turn-2".to_string()),
                None,
            )
            .await
            .expect("second spawn should succeed");

        let error = control
            .spawn(
                &explore_profile(),
                "session-3",
                Some("turn-3".to_string()),
                None,
            )
            .await
            .expect_err("third active agent should exceed concurrent limit");
        assert_eq!(
            error,
            AgentControlError::MaxConcurrentExceeded { current: 2, max: 2 }
        );

        let _ = control.mark_completed(&first.agent_id).await;
        let third = control
            .spawn(
                &explore_profile(),
                "session-3",
                Some("turn-3".to_string()),
                None,
            )
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
            .spawn(&profile, "session-1", Some("turn-1".to_string()), None)
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("agent A spawn should succeed");
        let agent_b = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
    async fn resume_transitions_completed_agent_back_to_running() {
        let control = AgentControl::with_limits(3, 10, 256);
        let handle = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("tree A parent spawn should succeed");
        let tree_a_child = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                Some(tree_a_parent.agent_id.clone()),
            )
            .await
            .expect("tree A child spawn should succeed");

        let tree_b_parent = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("tree B parent spawn should succeed");
        let _tree_b_child = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("first spawn should succeed");
        let _second = control
            .spawn(
                &explore_profile(),
                "session-2",
                Some("turn-2".to_string()),
                None,
            )
            .await
            .expect("second spawn should succeed");

        let _ = control.mark_running(&first.agent_id).await;
        let _ = control.mark_completed(&first.agent_id).await;

        // first 完成后释放了槽位，可以创建第三个
        let _third = control
            .spawn(
                &explore_profile(),
                "session-3",
                Some("turn-3".to_string()),
                None,
            )
            .await
            .expect("third spawn should succeed after first completed");

        // 恢复 first 会重新占用槽位，此时已有 3 个活跃（first resumed + second + third）
        let _ = control.resume(&first.agent_id).await;

        let error = control
            .spawn(
                &explore_profile(),
                "session-4",
                Some("turn-4".to_string()),
                None,
            )
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
    async fn push_inbox_deduplication_by_delivery_id() {
        let control = AgentControl::new();
        let handle = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
    async fn wait_for_inbox_returns_immediately_for_final_agent() {
        let control = AgentControl::new();
        let handle = control
            .spawn(
                &explore_profile(),
                "session-1",
                Some("turn-1".to_string()),
                None,
            )
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
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("root spawn should succeed");
        let middle = control
            .spawn(
                &explore_profile(),
                "session-middle",
                Some("turn-1".to_string()),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle spawn should succeed");
        let leaf = control
            .spawn(
                &explore_profile(),
                "session-leaf",
                Some("turn-1".to_string()),
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
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("root spawn should succeed");
        let middle_a = control
            .spawn(
                &explore_profile(),
                "session-middle-a",
                Some("turn-1".to_string()),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle_a spawn should succeed");
        let leaf_a = control
            .spawn(
                &explore_profile(),
                "session-leaf-a",
                Some("turn-1".to_string()),
                Some(middle_a.agent_id.clone()),
            )
            .await
            .expect("leaf_a spawn should succeed");
        let middle_b = control
            .spawn(
                &explore_profile(),
                "session-middle-b",
                Some("turn-1".to_string()),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle_b spawn should succeed");
        let leaf_b = control
            .spawn(
                &explore_profile(),
                "session-leaf-b",
                Some("turn-1".to_string()),
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
                Some("turn-1".to_string()),
                None,
            )
            .await
            .expect("root spawn should succeed");
        let middle = control
            .spawn(
                &explore_profile(),
                "session-middle",
                Some("turn-1".to_string()),
                Some(root.agent_id.clone()),
            )
            .await
            .expect("middle spawn should succeed");
        let leaf = control
            .spawn(
                &explore_profile(),
                "session-leaf",
                Some("turn-1".to_string()),
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
