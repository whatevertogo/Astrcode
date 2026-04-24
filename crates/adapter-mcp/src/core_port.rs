//! 桥接 `adapter-mcp` 与 `plugin-host::ResourceProvider`。
//!
//! 让 kernel 通过统一的资源端口读取 MCP 资源。
//! 通过 resource_index 将 URI 反查到所属服务器，再委托给 McpConnectionManager::read_resource。

use std::sync::Arc;

use astrcode_core::{AstrError, Result};
use astrcode_plugin_host::{ResourceProvider, ResourceReadResult, ResourceRequestContext};
use async_trait::async_trait;
use log::warn;
use serde_json::json;

use crate::manager::McpConnectionManager;

/// 基于 `McpConnectionManager` 的资源读取端口。
pub struct McpResourceProvider {
    manager: Arc<McpConnectionManager>,
}

impl McpResourceProvider {
    pub fn new(manager: Arc<McpConnectionManager>) -> Self {
        Self { manager }
    }
}

impl std::fmt::Debug for McpResourceProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpResourceProvider")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ResourceProvider for McpResourceProvider {
    async fn read_resource(
        &self,
        uri: &str,
        _context: &ResourceRequestContext,
    ) -> Result<ResourceReadResult> {
        // 从 resource_index 中查找 URI 所属服务器
        let surface = self.manager.current_surface().await;
        let server_name = surface
            .resource_index
            .iter()
            .find(|res| res.uri == uri)
            .map(|res| res.server_name.clone());

        let server_name = match server_name {
            Some(name) => name,
            None => {
                return Err(AstrError::Validation(format!(
                    "未找到 URI '{}' 对应的 MCP 服务器",
                    uri
                )));
            },
        };

        let content = self.manager.read_resource(&server_name, uri).await?;

        // 将 MCP 资源内容转为统一 ResourceReadResult
        // text 直接放入 content，blob 以 base64 编码形式保留
        let content_value = if let Some(text) = &content.text {
            json!({
                "text": text,
                "mimeType": content.mime_type,
            })
        } else if let Some(blob) = &content.blob {
            json!({
                "blob": blob,
                "mimeType": content.mime_type,
            })
        } else {
            warn!("MCP 资源 '{}' 内容为空（无 text 也无 blob）", uri);
            json!({ "mimeType": content.mime_type })
        };

        Ok(ResourceReadResult {
            uri: content.uri.clone(),
            content: content_value,
            metadata: json!({ "server": server_name }),
        })
    }
}
