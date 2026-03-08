use anyhow::{anyhow, Result};

use crate::config::{load_config, Profile};

pub struct AgentRuntime {}

impl AgentRuntime {
    pub fn new() -> Result<Self> {
        let config = load_config()?;
        let profile = select_profile(&config.profiles, &config.active_profile)?;
        profile.resolve_api_key()?;
        Ok(Self {})
    }
}

fn select_profile<'a>(profiles: &'a [Profile], active_profile: &str) -> Result<&'a Profile> {
    profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .or_else(|| profiles.first())
        .ok_or_else(|| anyhow!("no profiles configured"))
}
