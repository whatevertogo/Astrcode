use std::collections::HashMap;

use crate::ApplicationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConfigScope {
    User,
    Project,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransportConfig {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
    StreamableHttp {
        url: String,
        headers: HashMap<String, String>,
        oauth: Option<String>,
    },
    Sse {
        url: String,
        headers: HashMap<String, String>,
        oauth: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportConfig,
    pub scope: McpConfigScope,
    pub enabled: bool,
    pub timeout_secs: u64,
    pub init_timeout_secs: u64,
    pub max_reconnect_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerStatusSnapshot {
    pub name: String,
    pub scope: String,
    pub enabled: bool,
    pub state: String,
    pub error: Option<String>,
    pub tool_count: usize,
    pub prompt_count: usize,
    pub resource_count: usize,
    pub pending_approval: bool,
    pub server_signature: String,
}

#[derive(Debug, Clone, Default)]
pub struct McpService;

impl McpService {
    pub async fn list_status(&self) -> Vec<McpServerStatusSnapshot> {
        Vec::new()
    }

    pub async fn approve_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
        Ok(())
    }

    pub async fn reject_server(&self, _server_signature: &str) -> Result<(), ApplicationError> {
        Ok(())
    }

    pub async fn reconnect_server(&self, _name: &str) -> Result<(), ApplicationError> {
        Ok(())
    }

    pub async fn reset_project_choices(&self) -> Result<(), ApplicationError> {
        Ok(())
    }

    pub async fn upsert_config(&self, _config: McpServerConfig) -> Result<(), ApplicationError> {
        Ok(())
    }

    pub async fn remove_config(
        &self,
        _scope: McpConfigScope,
        _name: &str,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }

    pub async fn set_enabled(
        &self,
        _scope: McpConfigScope,
        _name: &str,
        _enabled: bool,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }
}
