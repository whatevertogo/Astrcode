//! # MCP Surface 快照与刷新辅助
//!
//! 负责把连接管理器中的运行时状态转换为 Astrcode 可消费的 surface 快照，
//! 并收口 tools/prompts/resources 的动态刷新逻辑。

use std::{collections::HashMap, sync::Arc};

use astrcode_core::CapabilityInvoker;
use astrcode_runtime_prompt::PromptDeclaration;
use log::{info, warn};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::{
    bridge::{
        prompt_bridge::collect_prompt_declarations, prompt_tool::McpPromptBridge,
        tool_bridge::McpToolBridge,
    },
    config::{McpConfigManager, McpConfigScope, McpServerConfig},
    manager::{McpManagedConnection, connection::McpConnectionState},
    protocol::{McpClient, types::*},
};

/// MCP surface 快照。
#[derive(Clone, Default)]
pub struct McpSurfaceSnapshot {
    pub capability_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub server_statuses: Vec<McpServerStatusSnapshot>,
    pub resource_index: Vec<McpIndexedResource>,
}

/// 供 runtime/server 消费的 MCP 服务器状态。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatusSnapshot {
    pub name: String,
    pub scope: String,
    pub enabled: bool,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tool_count: usize,
    pub prompt_count: usize,
    pub resource_count: usize,
    pub pending_approval: bool,
    pub server_signature: String,
}

/// MCP 资源索引项。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpIndexedResource {
    pub server_name: String,
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

pub(crate) fn build_prompt_declarations(
    server_name: &str,
    instructions: Option<&str>,
) -> Vec<PromptDeclaration> {
    collect_prompt_declarations(server_name, instructions)
}

pub(crate) fn build_server_invokers(
    server_name: &str,
    tools: &[McpToolInfo],
    prompts: &[McpPromptInfo],
    client: Arc<Mutex<McpClient>>,
) -> Vec<Arc<dyn CapabilityInvoker>> {
    let mut invokers: Vec<Arc<dyn CapabilityInvoker>> = Vec::new();
    let mut registered = std::collections::HashSet::new();

    for tool in tools {
        let bridge = McpToolBridge::new(
            server_name,
            &tool.name,
            tool.description.as_deref().unwrap_or(""),
            tool.input_schema
                .clone()
                .unwrap_or(serde_json::json!({"type": "object"})),
            tool.annotations.clone(),
            client.clone(),
        );
        let descriptor = bridge.descriptor();
        if registered.insert(descriptor.name.clone()) {
            invokers.push(Arc::new(bridge));
        } else {
            warn!(
                "MCP server '{}' skipped duplicate capability '{}'",
                server_name, descriptor.name
            );
        }
    }

    for prompt in prompts {
        let bridge = McpPromptBridge::new(server_name, prompt, client.clone());
        let descriptor = bridge.descriptor();
        if registered.insert(descriptor.name.clone()) {
            invokers.push(Arc::new(bridge));
        } else {
            warn!(
                "MCP server '{}' skipped prompt '{}' because capability '{}' already exists",
                server_name, prompt.name, descriptor.name
            );
        }
    }

    invokers
}

pub(crate) async fn refresh_tools_for_server(
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
    client: Arc<Mutex<McpClient>>,
    server_name: &str,
) {
    info!(
        "MCP server '{}' tools list changed, refreshing",
        server_name
    );

    let tools = {
        let locked = client.lock().await;
        match locked.list_tools().await {
            Ok(tools) => tools,
            Err(error) => {
                warn!(
                    "MCP server '{}' failed to refresh tools: {}",
                    server_name, error
                );
                return;
            },
        }
    };

    let mut conns = connections.lock().await;
    if let Some(managed) = conns.get_mut(server_name) {
        managed.tools = tools;
        managed.invokers = build_server_invokers(
            server_name,
            &managed.tools,
            &managed.prompts,
            client.clone(),
        );
        info!(
            "MCP server '{}' tools refreshed: {} tools, {} prompts",
            server_name,
            managed.tools.len(),
            managed.prompts.len()
        );
    }
}

pub(crate) async fn refresh_prompts_for_server(
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
    client: Arc<Mutex<McpClient>>,
    server_name: &str,
) {
    info!(
        "MCP server '{}' prompts list changed, refreshing",
        server_name
    );

    let prompts = {
        let locked = client.lock().await;
        match locked.list_prompts().await {
            Ok(prompts) => prompts,
            Err(error) => {
                warn!(
                    "MCP server '{}' failed to refresh prompts: {}",
                    server_name, error
                );
                return;
            },
        }
    };

    let mut conns = connections.lock().await;
    if let Some(managed) = conns.get_mut(server_name) {
        managed.prompts = prompts;
        managed.invokers = build_server_invokers(
            server_name,
            &managed.tools,
            &managed.prompts,
            client.clone(),
        );
        managed.prompt_declarations =
            build_prompt_declarations(server_name, managed.connection.instructions.as_deref());
        info!(
            "MCP server '{}' prompts refreshed: {} tools, {} prompts",
            server_name,
            managed.tools.len(),
            managed.prompts.len()
        );
    }
}

pub(crate) async fn refresh_resources_for_server(
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
    client: Arc<Mutex<McpClient>>,
    server_name: &str,
) {
    info!(
        "MCP server '{}' resources list changed, refreshing",
        server_name
    );

    let resources = {
        let locked = client.lock().await;
        match locked.list_resources().await {
            Ok(resources) => resources,
            Err(error) => {
                warn!(
                    "MCP server '{}' failed to refresh resources: {}",
                    server_name, error
                );
                return;
            },
        }
    };

    let mut conns = connections.lock().await;
    if let Some(managed) = conns.get_mut(server_name) {
        managed.resources = resources;
        info!(
            "MCP server '{}' resources refreshed: {} resources",
            server_name,
            managed.resources.len()
        );
    }
}

pub(crate) fn scope_name(scope: McpConfigScope) -> &'static str {
    match scope {
        McpConfigScope::User => "user",
        McpConfigScope::Project => "project",
        McpConfigScope::Local => "local",
    }
}

pub(crate) fn state_label(
    config: &McpServerConfig,
    connection: Option<&McpManagedConnection>,
    pending_approval: bool,
) -> (String, Option<String>) {
    if !config.enabled {
        return ("disabled".to_string(), None);
    }
    if pending_approval {
        return ("pending_approval".to_string(), None);
    }
    match connection.map(|managed| &managed.connection.state) {
        Some(McpConnectionState::Pending) | None => ("pending".to_string(), None),
        Some(McpConnectionState::Connecting) => ("connecting".to_string(), None),
        Some(McpConnectionState::Connected) => ("connected".to_string(), None),
        Some(McpConnectionState::NeedsAuth) => ("needs_auth".to_string(), None),
        Some(McpConnectionState::Disabled) => ("disabled".to_string(), None),
        Some(McpConnectionState::Failed(error)) => ("failed".to_string(), Some(error.clone())),
    }
}

pub(crate) fn build_server_status(
    config: &McpServerConfig,
    connection: Option<&McpManagedConnection>,
    pending_approval: bool,
) -> McpServerStatusSnapshot {
    let (state, error) = state_label(config, connection, pending_approval);
    McpServerStatusSnapshot {
        name: config.name.clone(),
        scope: scope_name(config.scope).to_string(),
        enabled: config.enabled,
        state,
        error,
        tool_count: connection
            .map(|managed| managed.tools.len())
            .unwrap_or_default(),
        prompt_count: connection
            .map(|managed| managed.prompts.len())
            .unwrap_or_default(),
        resource_count: connection
            .map(|managed| managed.resources.len())
            .unwrap_or_default(),
        pending_approval,
        server_signature: McpConfigManager::compute_signature(config),
    }
}
