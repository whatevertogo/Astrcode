use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, CancelToken, SubRunHandle,
};
use tokio::sync::watch;

use super::PendingParentDelivery;

/// agent 注册表的可变核心状态，受 `RwLock` 保护。
///
/// 所有写入操作通过 `*_locked` 后缀函数在持锁期间完成，
/// 读操作通过快照或 `watch` channel 实现无锁读取。
#[derive(Default)]
pub(super) struct AgentRegistryState {
    /// `sub_run_id → AgentEntry`，所有已注册 agent 的完整信息。
    pub(super) entries: HashMap<String, AgentEntry>,
    /// `agent_id → sub_run_id`，允许用 agent_id 反查 entry key。
    pub(super) agent_index: HashMap<String, String>,
    /// 当前占用 slot 的活跃 agent 数量（Pending/Running），受 capacity 限制。
    pub(super) active_count: usize,
    /// `parent_session_id → ParentDeliveryQueue`，child→parent 终态投递队列。
    pub(super) parent_delivery_queues: HashMap<String, ParentDeliveryQueue>,
}

pub(super) struct AgentEntry {
    pub(super) handle: SubRunHandle,
    pub(super) cancel: CancelToken,
    pub(super) status_tx: watch::Sender<AgentLifecycleStatus>,
    pub(super) parent_agent_id: Option<String>,
    pub(super) children: BTreeSet<String>,
    pub(super) finalized_seq: Option<u64>,
    /// 协作消息收件箱。send / child-delivery 产出信封存放在此。
    pub(super) inbox: VecDeque<AgentInboxEnvelope>,
    /// 收件箱版本号，每次 push_inbox 递增，用于 wait_for_inbox 的变化检测。
    pub(super) inbox_version: watch::Sender<u64>,
    /// 四工具模型的持久生命周期状态。
    /// Pending → Running → Idle → Terminated，完成单轮后不自动终止。
    pub(super) lifecycle_status: AgentLifecycleStatus,
    /// 最近一轮执行的结束原因。Running 期间为 None，turn 完成后设为 Some。
    pub(super) last_turn_outcome: Option<AgentTurnOutcome>,
}

/// delivery 在队列中的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingParentDeliveryState {
    /// 已入队，等待被 checkout。
    Queued,
    /// 已被 checkout，正在唤醒父 agent 消费。
    WakingParent,
}

/// 队列中的单条投递条目，携带状态以支持 checkout → consume/requeue 生命周期。
#[derive(Debug, Clone)]
pub(super) struct PendingParentDeliveryEntry {
    pub(super) delivery: PendingParentDelivery,
    pub(super) state: PendingParentDeliveryState,
}

/// 按 session 维度的 delivery 队列，FIFO 顺序，配合 dedup set 防止重复入队。
#[derive(Default)]
pub(super) struct ParentDeliveryQueue {
    pub(super) deliveries: VecDeque<PendingParentDeliveryEntry>,
    pub(super) known_delivery_ids: HashSet<String>,
}

/// 根据 sub_run_id 或 agent_id 解析到 entry 的主键（sub_run_id）。
/// entries 以 sub_run_id 为 key，agent_index 允许用 agent_id 反查。
pub(super) fn resolve_entry_key<'a>(
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

/// 返回指定 agent 的直接子节点 agent_id 列表。
pub(super) fn entry_children(state: &AgentRegistryState, agent_id: &str) -> Option<Vec<String>> {
    state
        .entries
        .get(agent_id)
        .map(|entry| entry.children.iter().cloned().collect())
}
