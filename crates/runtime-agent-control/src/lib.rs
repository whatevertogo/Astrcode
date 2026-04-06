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
    collections::{BTreeSet, HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use astrcode_core::{AgentProfile, AgentStatus, CancelToken, SubRunHandle, SubRunStorageMode};
use astrcode_runtime_config::{
    RuntimeConfig, resolve_agent_finalized_retain_limit, resolve_agent_max_concurrent,
    resolve_agent_max_subrun_depth,
};
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

    use astrcode_core::{AgentMode, AgentProfile, AgentStatus};
    use astrcode_runtime_config::{DEFAULT_MAX_AGENT_DEPTH, DEFAULT_MAX_CONCURRENT_AGENTS};

    use super::{AgentControl, AgentControlError};

    fn explore_profile() -> AgentProfile {
        AgentProfile {
            id: "explore".to_string(),
            name: "Explore".to_string(),
            description: "只读探索".to_string(),
            mode: AgentMode::SubAgent,
            system_prompt: None,
            allowed_tools: vec!["readFile".to_string()],
            disallowed_tools: Vec::new(),
            max_steps: Some(5),
            token_budget: Some(8_000),
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
}
