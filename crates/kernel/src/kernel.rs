use std::sync::Arc;

use astrcode_core::{
    AgentInboxEnvelope, AgentLifecycleStatus, AgentTurnOutcome, ChildSessionNotification,
    LlmProvider, PromptProvider, ResourceProvider, SubRunHandle,
};

use crate::{
    agent_tree::{AgentControl, AgentControlError, AgentControlLimits},
    error::KernelError,
    events::EventHub,
    gateway::KernelGateway,
    registry::CapabilityRouter,
    surface::SurfaceManager,
};

// ── 稳定控制合同类型 ───────────────────────────────────

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
        }
    }
}

/// 关闭子树的结果。
#[derive(Debug, Clone)]
pub struct CloseSubtreeResult {
    pub closed_agent_ids: Vec<String>,
}

// ── Kernel 主结构 ──────────────────────────────────────

#[derive(Clone)]
pub struct Kernel {
    gateway: KernelGateway,
    agent_control: AgentControl,
    surface: SurfaceManager,
    events: EventHub,
}

impl Kernel {
    pub fn builder() -> KernelBuilder {
        KernelBuilder::default()
    }

    pub fn gateway(&self) -> &KernelGateway {
        &self.gateway
    }

    pub fn agent_control(&self) -> &AgentControl {
        &self.agent_control
    }

    pub fn surface(&self) -> &SurfaceManager {
        &self.surface
    }

    pub fn events(&self) -> &EventHub {
        &self.events
    }

    // ── 稳定控制合同方法 ────────────────────────────────
    //
    // 这些方法是 kernel 对外暴露的稳定控制接口。
    // application/server 只能通过这些方法访问 agent 控制能力，
    // 不允许直接依赖 agent_tree 内部节点结构。

    /// 查询子运行状态（稳定视图）。
    pub async fn query_subrun_status(&self, agent_id: &str) -> Option<SubRunStatusView> {
        let handle = self.agent_control.get(agent_id).await?;
        Some(SubRunStatusView::from_handle(&handle))
    }

    /// 查询指定 session 的根 agent 状态。
    pub async fn query_root_agent_status(&self, session_id: &str) -> Option<SubRunStatusView> {
        let handle = self
            .agent_control
            .find_root_agent_for_session(session_id)
            .await?;
        Some(SubRunStatusView::from_handle(&handle))
    }

    /// 列出所有活跃 agent 的状态快照。
    pub async fn list_all_agent_statuses(&self) -> Vec<SubRunStatusView> {
        self.agent_control
            .list()
            .await
            .iter()
            .map(SubRunStatusView::from_handle)
            .collect()
    }

    /// 关闭指定 agent 及其子树。
    ///
    /// 返回被关闭的 agent ID 列表。
    pub async fn close_agent_subtree(
        &self,
        agent_id: &str,
    ) -> Result<CloseSubtreeResult, AgentControlError> {
        let handle = self.agent_control.terminate_subtree(agent_id).await.ok_or(
            AgentControlError::ParentAgentNotFound {
                agent_id: agent_id.to_string(),
            },
        )?;

        let mut closed = vec![handle.agent_id.clone()];
        let children = self.agent_control.collect_subtree_handles(agent_id).await;
        for child in children {
            closed.push(child.agent_id);
        }
        Ok(CloseSubtreeResult {
            closed_agent_ids: closed,
        })
    }

    /// 向 agent 收件箱投递消息。
    pub async fn deliver_to_agent(
        &self,
        agent_id: &str,
        envelope: AgentInboxEnvelope,
    ) -> Option<()> {
        self.agent_control.push_inbox(agent_id, envelope).await
    }

    /// 排空 agent 收件箱。
    pub async fn drain_agent_inbox(&self, agent_id: &str) -> Option<Vec<AgentInboxEnvelope>> {
        self.agent_control.drain_inbox(agent_id).await
    }

    /// 将子执行终止通知排入父会话的交付队列。
    pub async fn enqueue_child_delivery(
        &self,
        parent_session_id: &str,
        parent_turn_id: &str,
        notification: ChildSessionNotification,
    ) -> bool {
        self.agent_control
            .enqueue_parent_delivery(parent_session_id, parent_turn_id, notification)
            .await
    }

    /// 取消指定父 turn 下仍在运行的子执行。
    pub async fn cancel_subruns_for_turn(&self, parent_turn_id: &str) -> Vec<String> {
        self.agent_control
            .cancel_for_parent_turn(parent_turn_id)
            .await
            .into_iter()
            .map(|handle| handle.agent_id)
            .collect()
    }
}

#[derive(Default)]
pub struct KernelBuilder {
    capabilities: Option<CapabilityRouter>,
    llm: Option<Arc<dyn LlmProvider>>,
    prompt: Option<Arc<dyn PromptProvider>>,
    resource: Option<Arc<dyn ResourceProvider>>,
    agent_limits: Option<AgentControlLimits>,
    event_bus_capacity: Option<usize>,
}

impl KernelBuilder {
    pub fn with_capabilities(mut self, capabilities: CapabilityRouter) -> Self {
        self.capabilities = Some(capabilities);
        self
    }

    pub fn with_llm_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm = Some(provider);
        self
    }

    pub fn with_prompt_provider(mut self, provider: Arc<dyn PromptProvider>) -> Self {
        self.prompt = Some(provider);
        self
    }

    pub fn with_resource_provider(mut self, provider: Arc<dyn ResourceProvider>) -> Self {
        self.resource = Some(provider);
        self
    }

    pub fn with_agent_limits(mut self, limits: AgentControlLimits) -> Self {
        self.agent_limits = Some(limits);
        self
    }

    pub fn with_event_bus_capacity(mut self, capacity: usize) -> Self {
        self.event_bus_capacity = Some(capacity);
        self
    }

    pub fn build(self) -> Result<Kernel, KernelError> {
        let capabilities = self.capabilities.unwrap_or_default();
        let llm = self
            .llm
            .ok_or_else(|| KernelError::Validation("missing llm provider".to_string()))?;
        let prompt = self
            .prompt
            .ok_or_else(|| KernelError::Validation("missing prompt provider".to_string()))?;
        let resource = self
            .resource
            .ok_or_else(|| KernelError::Validation("missing resource provider".to_string()))?;

        let gateway = KernelGateway::new(capabilities.clone(), llm, prompt, resource);
        let events = EventHub::new(self.event_bus_capacity.unwrap_or(256));
        let surface = SurfaceManager::new();
        surface.replace_capabilities(&capabilities.invokers(), &events);

        Ok(Kernel {
            gateway,
            agent_control: AgentControl::from_limits(self.agent_limits.unwrap_or_default()),
            surface,
            events,
        })
    }
}
