use std::sync::Arc;

use astrcode_core::{LlmProvider, PromptProvider, ResourceProvider};

use crate::{
    agent_tree::{AgentControl, AgentControlLimits},
    error::KernelError,
    events::EventHub,
    gateway::KernelGateway,
    registry::CapabilityRouter,
    surface::SurfaceManager,
};

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
