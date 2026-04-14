use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentProfile, AgentTurnOutcome,
    ChildSessionNotification, DelegationMetadata, ResolvedExecutionLimitsSnapshot, SubRunHandle,
    SubRunStorageMode,
};

use crate::{
    agent_tree::{AgentControlError, PendingParentDelivery},
    kernel::Kernel,
};

/// 子运行稳定状态快照（不暴露内部树结构）。
#[derive(Debug, Clone)]
pub struct SubRunStatusView {
    pub sub_run_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub child_session_id: Option<String>,
    pub depth: usize,
    pub parent_agent_id: Option<String>,
    pub agent_profile: String,
    pub lifecycle: AgentLifecycleStatus,
    pub last_turn_outcome: Option<AgentTurnOutcome>,
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    pub delegation: Option<DelegationMetadata>,
}

impl SubRunStatusView {
    pub fn from_handle(handle: &SubRunHandle) -> Self {
        Self {
            sub_run_id: handle.sub_run_id.clone(),
            agent_id: handle.agent_id.clone(),
            session_id: handle.session_id.clone(),
            child_session_id: handle.child_session_id.clone(),
            depth: handle.depth,
            parent_agent_id: handle.parent_agent_id.clone(),
            agent_profile: handle.agent_profile.clone(),
            lifecycle: handle.lifecycle,
            last_turn_outcome: handle.last_turn_outcome,
            resolved_limits: handle.resolved_limits.clone(),
            delegation: handle.delegation.clone(),
        }
    }
}

/// 关闭子树的结果。
#[derive(Debug, Clone)]
pub struct CloseSubtreeResult {
    pub closed_agent_ids: Vec<String>,
}

/// Kernel 暴露给 application/server 的稳定 agent 控制面。
///
/// 这层只承载编排期真正需要的 agent 能力，避免调用方直接面向
/// `AgentControl` 内部树结构编程。
#[derive(Clone, Copy)]
pub struct KernelAgentSurface<'a> {
    kernel: &'a Kernel,
}

impl<'a> KernelAgentSurface<'a> {
    pub(crate) fn new(kernel: &'a Kernel) -> Self {
        Self { kernel }
    }

    /// 查询子运行状态（稳定视图）。
    pub async fn query_subrun_status(&self, agent_id: &str) -> Option<SubRunStatusView> {
        let handle = self.kernel.agent_control().get(agent_id).await?;
        Some(SubRunStatusView::from_handle(&handle))
    }

    /// 查询原始子运行句柄（供 application 编排层使用）。
    pub async fn get_handle(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.kernel.agent_control().get(sub_run_or_agent_id).await
    }

    /// 查询 agent 当前生命周期状态。
    pub async fn get_lifecycle(&self, sub_run_or_agent_id: &str) -> Option<AgentLifecycleStatus> {
        self.kernel
            .agent_control()
            .get_lifecycle(sub_run_or_agent_id)
            .await
    }

    /// 查询 agent 最近一轮执行结果。
    pub async fn get_turn_outcome(&self, sub_run_or_agent_id: &str) -> Option<AgentTurnOutcome> {
        self.kernel
            .agent_control()
            .get_turn_outcome(sub_run_or_agent_id)
            .await
            .flatten()
    }

    /// 在 finalized 可复用节点上启动新的执行轮次。
    pub async fn resume(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.kernel
            .agent_control()
            .resume(sub_run_or_agent_id)
            .await
    }

    /// 查询指定 session 的根 agent 状态。
    pub async fn query_root_status(&self, session_id: &str) -> Option<SubRunStatusView> {
        let handle = self
            .kernel
            .agent_control()
            .find_root_agent_for_session(session_id)
            .await?;
        Some(SubRunStatusView::from_handle(&handle))
    }

    /// 查询指定 session 的根 agent 原始句柄。
    pub async fn find_root_handle_for_session(&self, session_id: &str) -> Option<SubRunHandle> {
        self.kernel
            .agent_control()
            .find_root_agent_for_session(session_id)
            .await
    }

    /// 注册根 agent；如果已存在则返回既有句柄。
    pub async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> Result<SubRunHandle, AgentControlError> {
        self.kernel
            .agent_control()
            .register_root_agent(agent_id, session_id, profile_id)
            .await
    }

    /// 以独立 child session 模式创建新的子代理执行实例。
    pub async fn spawn_independent_child(
        &self,
        profile: &AgentProfile,
        session_id: impl Into<String>,
        child_session_id: String,
        parent_turn_id: String,
        parent_agent_id: String,
    ) -> Result<SubRunHandle, AgentControlError> {
        self.kernel
            .agent_control()
            .spawn_with_storage(
                profile,
                session_id,
                Some(child_session_id),
                parent_turn_id,
                Some(parent_agent_id),
                SubRunStorageMode::IndependentSession,
            )
            .await
    }

    /// 显式推进 agent 生命周期（仅限编排层需要的少量控制面操作）。
    pub async fn set_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
        new_status: AgentLifecycleStatus,
    ) -> Option<()> {
        self.kernel
            .agent_control()
            .set_lifecycle(sub_run_or_agent_id, new_status)
            .await
    }

    pub async fn set_resolved_limits(
        &self,
        sub_run_or_agent_id: &str,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
    ) -> Option<()> {
        self.kernel
            .agent_control()
            .set_resolved_limits(sub_run_or_agent_id, resolved_limits)
            .await
    }

    pub async fn set_delegation(
        &self,
        sub_run_or_agent_id: &str,
        delegation: Option<DelegationMetadata>,
    ) -> Option<()> {
        self.kernel
            .agent_control()
            .set_delegation(sub_run_or_agent_id, delegation)
            .await
    }

    /// 列出所有 agent 的状态快照。
    pub async fn list_statuses(&self) -> Vec<SubRunStatusView> {
        self.kernel
            .agent_control()
            .list()
            .await
            .iter()
            .map(SubRunStatusView::from_handle)
            .collect()
    }

    /// 统计某个 parent turn 下已经派生出的 child 数量。
    ///
    /// 这是编排层的 spawn budget 查询，不暴露底层树节点结构，只返回当前需要的计数结果。
    pub async fn count_children_spawned_for_turn(
        &self,
        parent_agent_id: &str,
        parent_turn_id: &str,
    ) -> usize {
        self.kernel
            .agent_control()
            .list()
            .await
            .into_iter()
            .filter(|handle| {
                handle.parent_turn_id == parent_turn_id
                    && handle.parent_agent_id.as_deref() == Some(parent_agent_id)
            })
            .count()
    }

    /// 关闭指定 agent 及其子树，并返回被关闭的 agent id 列表。
    pub async fn close_subtree(
        &self,
        agent_id: &str,
    ) -> Result<CloseSubtreeResult, AgentControlError> {
        let closed_agent_ids = self
            .kernel
            .agent_control()
            .terminate_subtree_and_collect_handles(agent_id)
            .await
            .map(|handles| handles.into_iter().map(|handle| handle.agent_id).collect())
            .ok_or(AgentControlError::ParentAgentNotFound {
                agent_id: agent_id.to_string(),
            })?;
        Ok(CloseSubtreeResult { closed_agent_ids })
    }

    /// 向 agent inbox 推送一条信封（用于 send 工具的 durable queue 路径）。
    pub async fn deliver(&self, agent_id: &str, envelope: AgentInboxEnvelope) -> Option<()> {
        self.kernel
            .agent_control()
            .push_inbox(agent_id, envelope)
            .await
    }

    /// 一次性取出 inbox 中所有待处理信封并清空（用于 idle agent resume 时合并消息）。
    pub async fn drain_inbox(&self, agent_id: &str) -> Option<Vec<AgentInboxEnvelope>> {
        self.kernel.agent_control().drain_inbox(agent_id).await
    }

    /// 收集以指定 agent 为根的整棵子树 handle（用于 close 前的 durable discard 批量标记）。
    pub async fn collect_subtree_handles(&self, sub_run_or_agent_id: &str) -> Vec<SubRunHandle> {
        self.kernel
            .agent_control()
            .collect_subtree_handles(sub_run_or_agent_id)
            .await
    }

    /// 终止子树但不收集 handle（用于内部 cancel 路径，不需要返回被终止列表）。
    pub async fn terminate_subtree(&self, sub_run_or_agent_id: &str) -> Option<SubRunHandle> {
        self.kernel
            .agent_control()
            .terminate_subtree(sub_run_or_agent_id)
            .await
    }

    /// 将 child terminal delivery 排入父 session 的 delivery queue。
    /// 返回 true 表示入队成功，false 表示容量已满或重复 delivery_id。
    pub async fn enqueue_child_delivery(
        &self,
        parent_session_id: impl Into<String>,
        parent_turn_id: impl Into<String>,
        notification: ChildSessionNotification,
    ) -> bool {
        self.kernel
            .agent_control()
            .enqueue_parent_delivery(parent_session_id, parent_turn_id, notification)
            .await
    }

    /// 从队列头部批量 checkout 同一 parent_agent_id 的连续 delivery（状态 Queued → WakingParent）。
    pub async fn checkout_parent_delivery_batch(
        &self,
        parent_session_id: &str,
    ) -> Option<Vec<PendingParentDelivery>> {
        self.kernel
            .agent_control()
            .checkout_parent_delivery_batch(parent_session_id)
            .await
    }

    /// wake 失败时将 delivery 重新标记为 Queued，等待下次 retry。
    pub async fn requeue_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) {
        self.kernel
            .agent_control()
            .requeue_parent_delivery_batch(parent_session_id, delivery_ids)
            .await;
    }

    /// wake 成功后从队列中移除已消费的 delivery；队列为空时清理整个 session 条目。
    pub async fn consume_parent_delivery_batch(
        &self,
        parent_session_id: &str,
        delivery_ids: &[String],
    ) -> bool {
        self.kernel
            .agent_control()
            .consume_parent_delivery_batch(parent_session_id, delivery_ids)
            .await
    }

    /// 取消指定 parent turn 下所有仍在运行的子 agent（用于 turn 结束时的级联清理）。
    pub async fn cancel_subruns_for_turn(&self, parent_turn_id: &str) -> Vec<String> {
        self.kernel
            .agent_control()
            .cancel_for_parent_turn(parent_turn_id)
            .await
            .into_iter()
            .map(|handle| handle.agent_id)
            .collect()
    }
}

impl Kernel {
    pub fn agent(&self) -> KernelAgentSurface<'_> {
        KernelAgentSurface::new(self)
    }
}
