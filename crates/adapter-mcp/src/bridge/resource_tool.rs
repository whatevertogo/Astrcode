//! # MCP 资源工具
//!
//! 内置工具 `ListMcpResources` 和 `ReadMcpResource`，
//! 让 Agent 查询和读取已连接 MCP 服务器提供的资源。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use astrcode_core::{
    CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, CapabilityKind,
    CapabilitySpec, Result, maybe_persist_tool_result,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use log::warn;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::manager::{McpManagedConnection, connection::McpConnectionState};

/// 列出所有 MCP 服务器资源的内置工具。
///
/// 遍历所有已连接的服务器调用 `resources/list`，
/// 返回资源 URI、名称和描述。
pub struct ListMcpResourcesTool {
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
}

impl ListMcpResourcesTool {
    pub(crate) fn new(connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>) -> Self {
        Self { connections }
    }
}

#[async_trait]
impl CapabilityInvoker for ListMcpResourcesTool {
    fn capability_spec(&self) -> CapabilitySpec {
        CapabilitySpec::builder("mcp_list_resources", CapabilityKind::Tool)
            .description("列出所有已连接 MCP 服务器提供的资源")
            .schema(
                json!({"type": "object", "properties": {}}),
                json!({"type": "string"}),
            )
            .concurrency_safe(true)
            .tags(["source:mcp"])
            .build()
            .expect("mcp_list_resources capability spec must build")
    }

    async fn invoke(
        &self,
        _payload: Value,
        _ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let start = Instant::now();
        let conns = self.connections.lock().await;
        let mut all_resources = Vec::new();

        for (name, managed) in conns.iter() {
            if !matches!(managed.connection.state, McpConnectionState::Connected) {
                continue;
            }
            let client = managed.client.lock().await;
            match client.list_resources().await {
                Ok(resources) => {
                    for res in resources {
                        all_resources.push(json!({
                            "server": name,
                            "uri": res.uri,
                            "name": res.name,
                            "description": res.description,
                            "mimeType": res.mime_type,
                        }));
                    }
                },
                Err(e) => {
                    warn!("MCP server '{}' list_resources failed: {}", name, e);
                },
            }
        }

        let output = if all_resources.is_empty() {
            json!("No MCP resources available")
        } else {
            json!(all_resources)
        };

        let mut result = CapabilityExecutionResult::ok("mcp_list_resources", output);
        result.duration_ms = start.elapsed().as_millis() as u64;
        Ok(result)
    }
}

/// 读取指定 MCP 服务器资源的内置工具。
///
/// 需要指定服务器名和资源 URI，调用 `resources/read`。
pub struct ReadMcpResourceTool {
    connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>,
}

impl ReadMcpResourceTool {
    pub(crate) fn new(connections: Arc<Mutex<HashMap<String, McpManagedConnection>>>) -> Self {
        Self { connections }
    }
}

#[async_trait]
impl CapabilityInvoker for ReadMcpResourceTool {
    fn capability_spec(&self) -> CapabilitySpec {
        CapabilitySpec::builder("mcp_read_resource", CapabilityKind::Tool)
            .description("从指定 MCP 服务器读取资源内容")
            .schema(
                json!({
                    "type": "object",
                    "properties": {
                        "server": { "type": "string" },
                        "uri": { "type": "string" }
                    },
                    "required": ["server", "uri"]
                }),
                json!({"type": "string"}),
            )
            .tags(["source:mcp"])
            .build()
            .expect("mcp_read_resource capability spec must build")
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let start = Instant::now();
        let server_name = payload.get("server").and_then(|v| v.as_str()).unwrap_or("");
        let uri = payload.get("uri").and_then(|v| v.as_str()).unwrap_or("");

        if server_name.is_empty() || uri.is_empty() {
            let mut result = CapabilityExecutionResult::failure(
                "mcp_read_resource",
                "Missing required parameters: server and uri",
                json!(null),
            );
            result.duration_ms = start.elapsed().as_millis() as u64;
            return Ok(result);
        }

        let conns = self.connections.lock().await;
        let managed = match conns.get(server_name) {
            Some(m) => m,
            None => {
                let mut result = CapabilityExecutionResult::failure(
                    "mcp_read_resource",
                    format!("MCP server '{}' not found", server_name),
                    json!(null),
                );
                result.duration_ms = start.elapsed().as_millis() as u64;
                return Ok(result);
            },
        };

        let client = managed.client.lock().await;
        match client.read_resource(uri).await {
            Ok(content) => {
                let output = if let Some(text) = &content.text {
                    let session_dir = session_dir_for_mcp_results(ctx)?;
                    let rendered = maybe_persist_tool_result(
                        &session_dir,
                        "mcp_read_resource",
                        text,
                        32 * 1024,
                    );
                    json!({
                        "uri": content.uri,
                        "mimeType": content.mime_type,
                        "text": rendered.output,
                    })
                } else if let Some(blob) = &content.blob {
                    match persist_blob_content(
                        ctx,
                        &content.uri,
                        content.mime_type.as_deref(),
                        blob,
                    ) {
                        Ok(relative_path) => json!({
                            "uri": content.uri,
                            "mimeType": content.mime_type,
                            "filePath": relative_path,
                        }),
                        Err(error) => {
                            let mut result = CapabilityExecutionResult::failure(
                                "mcp_read_resource",
                                format!("Failed to persist binary resource '{}': {}", uri, error),
                                json!(null),
                            );
                            result.duration_ms = start.elapsed().as_millis() as u64;
                            return Ok(result);
                        },
                    }
                } else {
                    json!({
                        "uri": content.uri,
                        "mimeType": content.mime_type,
                    })
                };
                let mut result = CapabilityExecutionResult::ok("mcp_read_resource", output);
                result.duration_ms = start.elapsed().as_millis() as u64;
                Ok(result)
            },
            Err(e) => {
                warn!(
                    "MCP read_resource '{}' on '{}' failed: {}",
                    uri, server_name, e
                );
                let mut result = CapabilityExecutionResult::failure(
                    "mcp_read_resource",
                    format!(
                        "Failed to read resource '{}' from '{}': {}",
                        uri, server_name, e
                    ),
                    json!(null),
                );
                result.duration_ms = start.elapsed().as_millis() as u64;
                Ok(result)
            },
        }
    }
}

fn session_dir_for_mcp_results(ctx: &CapabilityContext) -> Result<PathBuf> {
    let project_dir = astrcode_core::project::project_dir(&ctx.working_dir).map_err(|error| {
        astrcode_core::AstrError::Internal(format!(
            "failed to resolve project directory for '{}': {}",
            ctx.working_dir.display(),
            error
        ))
    })?;
    Ok(project_dir
        .join("sessions")
        .join(ctx.session_id.to_string()))
}

fn persist_blob_content(
    ctx: &CapabilityContext,
    uri: &str,
    mime_type: Option<&str>,
    blob: &str,
) -> Result<String> {
    let session_dir = session_dir_for_mcp_results(ctx)?;
    let bytes = STANDARD.decode(blob).map_err(|error| {
        astrcode_core::AstrError::Validation(format!(
            "invalid base64 blob for resource '{}': {}",
            uri, error
        ))
    })?;
    let file_name = sanitize_resource_name(uri, mime_type);
    let relative_path = format!("tool-results/{}", file_name);
    let target_path = session_dir.join(Path::new(&relative_path));
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            astrcode_core::AstrError::io(
                format!(
                    "failed to create MCP resource directory '{}'",
                    parent.display()
                ),
                error,
            )
        })?;
    }
    std::fs::write(&target_path, bytes).map_err(|error| {
        astrcode_core::AstrError::io(
            format!("failed to persist MCP resource '{}'", target_path.display()),
            error,
        )
    })?;
    Ok(relative_path.replace('\\', "/"))
}

fn sanitize_resource_name(uri: &str, mime_type: Option<&str>) -> String {
    let stem: String = uri
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(48)
        .collect();
    let stem = if stem.is_empty() {
        "mcp-resource".to_string()
    } else {
        stem
    };
    let ext = match mime_type {
        Some("application/json") => "json",
        Some("text/plain") => "txt",
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/svg+xml") => "svg",
        Some("application/pdf") => "pdf",
        _ => "bin",
    };
    format!("{stem}.{ext}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn test_list_resources_capability_spec() {
        let conns = Arc::new(Mutex::new(HashMap::new()));
        let tool = ListMcpResourcesTool::new(conns);
        let spec = tool.capability_spec();
        assert_eq!(spec.name.as_str(), "mcp_list_resources");
    }

    #[test]
    fn test_read_resource_capability_spec() {
        let conns = Arc::new(Mutex::new(HashMap::new()));
        let tool = ReadMcpResourceTool::new(conns);
        let spec = tool.capability_spec();
        assert_eq!(spec.name.as_str(), "mcp_read_resource");
    }
}
