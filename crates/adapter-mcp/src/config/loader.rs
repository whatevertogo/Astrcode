//! # MCP 配置加载器
//!
//! 从 `.mcp.json` 和 settings 加载 MCP 服务器配置。
//! 支持环境变量展开（`${VAR}`）和签名去重。

use std::{collections::HashMap, path::Path};

use astrcode_core::{AstrError, Result};
use log::warn;
use serde_json::Value;

use super::types::{
    McpConfigScope, McpJsonFile, McpJsonServerEntry, McpServerConfig, McpTransportConfig,
};

/// MCP 配置管理器。
///
/// 从多个来源加载 MCP 服务器配置，处理环境变量展开和签名去重。
pub struct McpConfigManager;

impl McpConfigManager {
    /// 从 `.mcp.json` 文件内容解析服务器配置。
    ///
    /// 环境变量引用 `${VAR}` 会被展开为实际值，
    /// 缺失的环境变量返回错误并明确指出缺失的变量名。
    pub fn load_from_json(content: &str, scope: McpConfigScope) -> Result<Vec<McpServerConfig>> {
        let json_file: McpJsonFile =
            serde_json::from_str(content).map_err(|e| AstrError::parse(".mcp.json", e))?;
        Self::load_from_file_model(json_file, scope)
    }

    /// 从原始 JSON value 解析服务器配置。
    ///
    /// `runtime-config` 只把 `mcp` 字段作为原始 JSON 承载，因此这里提供 value 入口，
    /// 避免 runtime 再把 JSON 序列化回字符串。
    pub fn load_from_value(value: &Value, scope: McpConfigScope) -> Result<Vec<McpServerConfig>> {
        let json_file: McpJsonFile = serde_json::from_value(value.clone())
            .map_err(|e| AstrError::parse("mcp config value", e))?;
        Self::load_from_file_model(json_file, scope)
    }

    fn load_from_file_model(
        json_file: McpJsonFile,
        scope: McpConfigScope,
    ) -> Result<Vec<McpServerConfig>> {
        let mut configs = Vec::new();
        let mut seen_signatures = HashMap::new();

        for (name, entry) in &json_file.mcp_servers {
            // 解析环境变量（包括名称中的 ${VAR}）
            let resolved_name = expand_env_vars(name)?;

            let config = Self::entry_to_config(&resolved_name, entry, scope)?;

            // 签名去重：stdio 按 command:args，远程按 URL
            let signature = Self::compute_signature(&config);
            if let Some(existing) = seen_signatures.get(&signature) {
                warn!(
                    "MCP config dedup: '{}' skipped, duplicate signature of '{}'",
                    resolved_name, existing
                );
                continue;
            }
            seen_signatures.insert(signature, resolved_name.clone());

            configs.push(config);
        }

        Ok(configs)
    }

    /// 从文件路径加载配置。
    pub fn load_from_file(path: &Path, scope: McpConfigScope) -> Result<Vec<McpServerConfig>> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| AstrError::io(format!("read MCP config: {}", path.display()), e))?;
        Self::load_from_json(&content, scope)
    }

    /// 将 `.mcp.json` 条目转换为 McpServerConfig。
    fn entry_to_config(
        name: &str,
        entry: &McpJsonServerEntry,
        scope: McpConfigScope,
    ) -> Result<McpServerConfig> {
        // 展开环境变量
        let command = entry.command.as_deref().map(expand_env_vars).transpose()?;
        let url = entry.url.as_deref().map(expand_env_vars).transpose()?;
        let args = entry
            .args
            .as_ref()
            .map(|args| {
                args.iter()
                    .map(|a| expand_env_vars(a))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        let env = entry
            .env
            .as_ref()
            .map(|env| {
                env.iter()
                    .map(|(k, v)| expand_env_vars(v).map(|v| (k.clone(), v)))
                    .collect::<Result<HashMap<_, _>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let headers = entry
            .headers
            .as_ref()
            .map(|h| {
                h.iter()
                    .map(|(k, v)| expand_env_vars(v).map(|v| (k.clone(), v)))
                    .collect::<Result<HashMap<_, _>>>()
            })
            .transpose()?;

        // 推断传输类型
        let transport = if let Some(transport_type) = &entry.transport_type {
            match transport_type.as_str() {
                "stdio" => {
                    let command = command.ok_or_else(|| {
                        AstrError::Validation(format!(
                            "MCP server '{}' declared as stdio but missing command",
                            name
                        ))
                    })?;
                    McpTransportConfig::Stdio {
                        command,
                        args: args.unwrap_or_default(),
                        env,
                    }
                },
                "http" => {
                    let url = url.ok_or_else(|| {
                        AstrError::Validation(format!(
                            "MCP server '{}' declared as HTTP but missing url",
                            name
                        ))
                    })?;
                    McpTransportConfig::StreamableHttp {
                        url,
                        headers: headers.unwrap_or_default(),
                        oauth: None,
                    }
                },
                "sse" => {
                    let url = url.ok_or_else(|| {
                        AstrError::Validation(format!(
                            "MCP server '{}' declared as SSE but missing url",
                            name
                        ))
                    })?;
                    McpTransportConfig::Sse {
                        url,
                        headers: headers.unwrap_or_default(),
                        oauth: None,
                    }
                },
                other => {
                    return Err(AstrError::Validation(format!(
                        "MCP server '{}' has unknown transport type: '{}'",
                        name, other
                    )));
                },
            }
        } else if let Some(command) = command {
            // 有 command 字段时推断为 stdio
            McpTransportConfig::Stdio {
                command,
                args: args.unwrap_or_default(),
                env,
            }
        } else if let Some(url) = url {
            // 有 url 字段但无 type 时默认为 http
            McpTransportConfig::StreamableHttp {
                url,
                headers: headers.unwrap_or_default(),
                oauth: None,
            }
        } else {
            return Err(AstrError::Validation(format!(
                "MCP server '{}' has neither 'command' nor 'url' specified",
                name
            )));
        };

        Ok(McpServerConfig {
            name: name.to_string(),
            transport,
            scope,
            enabled: !entry.disabled.unwrap_or(false),
            timeout_secs: entry.timeout.unwrap_or(120),
            init_timeout_secs: entry.init_timeout.unwrap_or(30),
            max_reconnect_attempts: entry.max_reconnect_attempts.unwrap_or(5),
        })
    }

    /// 计算配置签名用于去重。
    /// 计算服务器签名（用于审批和去重）。
    ///
    /// stdio 按 `command:args`，远程按 URL。
    pub fn compute_signature(config: &McpServerConfig) -> String {
        match &config.transport {
            McpTransportConfig::Stdio { command, args, .. } => {
                format!("stdio:{}:{}", command, args.join(","))
            },
            McpTransportConfig::StreamableHttp { url, .. } => format!("http:{}", url),
            McpTransportConfig::Sse { url, .. } => format!("sse:{}", url),
        }
    }
}

/// 展开字符串中的 `${VAR}` 环境变量引用。
///
/// 缺失的环境变量返回错误并明确指出变量名。
fn expand_env_vars(input: &str) -> Result<String> {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // 消耗 '{'
            let mut var_name = String::new();
            let mut found_close = false;

            for inner_ch in chars.by_ref() {
                if inner_ch == '}' {
                    found_close = true;
                    break;
                }
                var_name.push(inner_ch);
            }

            if !found_close {
                return Err(AstrError::Validation(format!(
                    "unclosed ${{...}} in '{}'",
                    input
                )));
            }

            if var_name.is_empty() {
                return Err(AstrError::Validation(format!("empty ${{}} in '{}'", input)));
            }

            let value = std::env::var(&var_name).map_err(|_| {
                AstrError::Validation(format!(
                    "environment variable '{}' not found (referenced in '{}')",
                    var_name, input
                ))
            })?;
            result.push_str(&value);
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_from_json_stdio() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"],
                    "env": {}
                }
            }
        }"#;

        let configs = McpConfigManager::load_from_json(json, McpConfigScope::Project).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "filesystem");
        assert!(matches!(
            configs[0].transport,
            McpTransportConfig::Stdio { .. }
        ));
        assert!(configs[0].enabled);
    }

    #[test]
    fn test_load_from_json_explicit_stdio_type() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"],
                    "env": {"NODE_ENV": "test"}
                }
            }
        }"#;

        let configs = McpConfigManager::load_from_json(json, McpConfigScope::Project).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "filesystem");
        assert!(matches!(
            &configs[0].transport,
            McpTransportConfig::Stdio { command, args, env }
                if command == "npx"
                    && args == &vec!["-y".to_string(), "@modelcontextprotocol/server-filesystem".to_string()]
                    && env.get("NODE_ENV") == Some(&"test".to_string())
        ));
    }

    #[test]
    fn test_explicit_stdio_requires_command() {
        let json = r#"{
            "mcpServers": {
                "bad": {
                    "type": "stdio",
                    "args": ["server"]
                }
            }
        }"#;

        let result = McpConfigManager::load_from_json(json, McpConfigScope::Project);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("declared as stdio but missing command")
        );
    }

    #[test]
    fn test_load_from_json_http() {
        let json = r#"{
            "mcpServers": {
                "remote-api": {
                    "type": "http",
                    "url": "http://localhost:8080/mcp",
                    "headers": {"Authorization": "Bearer token"}
                }
            }
        }"#;

        let configs = McpConfigManager::load_from_json(json, McpConfigScope::User).unwrap();
        assert_eq!(configs.len(), 1);
        assert!(matches!(
            configs[0].transport,
            McpTransportConfig::StreamableHttp { .. }
        ));
    }

    #[test]
    fn test_load_from_json_disabled() {
        let json = r#"{
            "mcpServers": {
                "disabled-server": {
                    "command": "test",
                    "disabled": true
                }
            }
        }"#;

        let configs = McpConfigManager::load_from_json(json, McpConfigScope::Project).unwrap();
        assert_eq!(configs.len(), 1);
        assert!(!configs[0].enabled);
    }

    #[test]
    fn test_dedup_same_signature() {
        let json = r#"{
            "mcpServers": {
                "fs1": { "command": "npx", "args": ["-y", "fs-server"] },
                "fs2": { "command": "npx", "args": ["-y", "fs-server"] }
            }
        }"#;

        let configs = McpConfigManager::load_from_json(json, McpConfigScope::Project).unwrap();
        assert_eq!(configs.len(), 1); // 第二个被去重
        // HashMap 迭代顺序不确定，保留的是先遇到的那个
        assert!(configs[0].name == "fs1" || configs[0].name == "fs2");
    }

    #[test]
    fn test_no_command_no_url_error() {
        let json = r#"{
            "mcpServers": {
                "bad": { "args": ["something"] }
            }
        }"#;

        let result = McpConfigManager::load_from_json(json, McpConfigScope::Project);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("neither"));
    }

    #[test]
    fn test_unknown_transport_type_error() {
        let json = r#"{
            "mcpServers": {
                "bad": { "type": "websocket", "url": "ws://localhost" }
            }
        }"#;

        let result = McpConfigManager::load_from_json(json, McpConfigScope::Project);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unknown transport type")
        );
    }

    #[test]
    fn test_missing_env_var_error() {
        let json = r#"{
            "mcpServers": {
                "env-test": {
                    "command": "${NONEXISTENT_VAR_12345}",
                    "args": []
                }
            }
        }"#;

        let result = McpConfigManager::load_from_json(json, McpConfigScope::Project);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("NONEXISTENT_VAR_12345"), "Error: {}", err);
    }

    #[test]
    fn test_expand_env_vars_with_existing_var() {
        std::env::set_var("TEST_MCP_HOME", "/test/path");
        let result = expand_env_vars("${TEST_MCP_HOME}/server").unwrap();
        assert_eq!(result, "/test/path/server");
        std::env::remove_var("TEST_MCP_HOME");
    }

    #[test]
    fn test_expand_env_vars_no_vars() {
        let result = expand_env_vars("plain-string").unwrap();
        assert_eq!(result, "plain-string");
    }

    #[test]
    fn test_expand_env_vars_unclosed() {
        let result = expand_env_vars("${UNCLOSED");
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_signature() {
        let config = McpServerConfig {
            name: "test".to_string(),
            transport: McpTransportConfig::Stdio {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "server".to_string()],
                env: HashMap::new(),
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };
        let sig = McpConfigManager::compute_signature(&config);
        assert_eq!(sig, "stdio:npx:-y,server");
    }
}
