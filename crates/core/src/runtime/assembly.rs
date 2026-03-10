use anyhow::Result;

use crate::agent_loop::AgentLoop;
use crate::provider_factory::ConfigFileProviderFactory;
use crate::tools::registry::ToolRegistry;

pub(crate) fn build_agent_loop() -> Result<AgentLoop> {
    let tools = ToolRegistry::with_v1_defaults();
    Ok(AgentLoop::new(
        std::sync::Arc::new(ConfigFileProviderFactory),
        tools,
    ))
}
