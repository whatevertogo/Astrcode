use astrcode_core::AgentRuntime;

pub struct AgentHandle {
    pub runtime: AgentRuntime,
}

impl AgentHandle {
    pub fn new() -> anyhow::Result<Self> {
        let runtime = AgentRuntime::new()?;
        Ok(Self { runtime })
    }
}
