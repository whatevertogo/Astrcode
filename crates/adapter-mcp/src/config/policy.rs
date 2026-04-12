//! # MCP 策略过滤器
//!
//! 按允许/拒绝列表过滤 MCP 服务器配置。
//! 拒绝列表优先于允许列表。

use super::types::{McpServerConfig, McpTransportConfig};

/// MCP 策略过滤器。
///
/// 支持按名称、命令、URL 匹配进行允许/拒绝过滤。
/// 拒绝规则优先于允许规则：匹配拒绝列表的配置直接排除，
/// 匹配允许列表的配置通过，其余（不在任何列表中）被排除。
pub struct McpPolicyFilter {
    /// 允许的服务器名称/模式列表。
    allow_list: Vec<String>,
    /// 拒绝的服务器名称/模式列表。
    deny_list: Vec<String>,
}

impl Default for McpPolicyFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl McpPolicyFilter {
    /// 创建空策略（允许所有）。
    pub fn new() -> Self {
        Self {
            allow_list: Vec::new(),
            deny_list: Vec::new(),
        }
    }

    /// 创建带允许和拒绝列表的策略。
    pub fn with_lists(allow_list: Vec<String>, deny_list: Vec<String>) -> Self {
        Self {
            allow_list,
            deny_list,
        }
    }

    /// 过滤服务器配置列表。
    ///
    /// 规则：
    /// 1. 拒绝列表优先——匹配拒绝列表的配置被排除
    /// 2. 允许列表为空时，未被拒绝的都通过
    /// 3. 允许列表非空时，只有匹配允许列表的才通过
    pub fn filter(&self, configs: Vec<McpServerConfig>) -> Vec<McpServerConfig> {
        configs
            .into_iter()
            .filter(|config| self.is_allowed(config))
            .collect()
    }

    /// 检查单个配置是否被策略允许。
    pub fn is_allowed(&self, config: &McpServerConfig) -> bool {
        let identifiers = self.extract_identifiers(config);

        // 拒绝列表优先
        for id in &identifiers {
            if self.matches_list(id, &self.deny_list) {
                return false;
            }
        }

        // 允许列表为空时默认允许（未被拒绝即可）
        if self.allow_list.is_empty() {
            return true;
        }

        // 检查是否匹配允许列表
        for id in &identifiers {
            if self.matches_list(id, &self.allow_list) {
                return true;
            }
        }

        false
    }

    /// 提取配置的标识符（名称 + 命令/URL）。
    fn extract_identifiers(&self, config: &McpServerConfig) -> Vec<String> {
        let mut ids = vec![config.name.clone()];
        match &config.transport {
            McpTransportConfig::Stdio { command, args, .. } => {
                ids.push(command.clone());
                ids.push(format!("{}:{}", command, args.join(",")));
            },
            McpTransportConfig::StreamableHttp { url, .. } => {
                ids.push(url.clone());
            },
            McpTransportConfig::Sse { url, .. } => {
                ids.push(url.clone());
            },
        }
        ids
    }

    /// 检查标识符是否匹配列表中的任一模式。
    fn matches_list(&self, identifier: &str, list: &[String]) -> bool {
        list.iter().any(|pattern| {
            if pattern.contains('*') {
                glob_match(pattern, identifier)
            } else {
                identifier == pattern
            }
        })
    }
}

/// 简单的 glob 匹配（支持 * 通配符）。
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }

    let mut text = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            // 第一段必须前缀匹配
            if !text.starts_with(part) {
                return false;
            }
            text = &text[part.len()..];
        } else if i == parts.len() - 1 {
            // 最后一段必须后缀匹配
            return text.ends_with(part);
        } else {
            // 中间段包含匹配
            match text.find(part) {
                Some(pos) => text = &text[pos + part.len()..],
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::McpConfigScope;

    fn stdio_config(name: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransportConfig::Stdio {
                command: command.to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        }
    }

    fn http_config(name: &str, url: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransportConfig::StreamableHttp {
                url: url.to_string(),
                headers: HashMap::new(),
                oauth: None,
            },
            scope: McpConfigScope::Project,
            enabled: true,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        }
    }

    #[test]
    fn test_empty_filter_allows_all() {
        let filter = McpPolicyFilter::new();
        let configs = vec![stdio_config("a", "cmd-a"), stdio_config("b", "cmd-b")];
        let result = filter.filter(configs);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_deny_list_excludes() {
        let filter = McpPolicyFilter::with_lists(Vec::new(), vec!["dangerous".to_string()]);
        let configs = vec![
            stdio_config("safe", "cmd-safe"),
            stdio_config("dangerous", "cmd-danger"),
        ];
        let result = filter.filter(configs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "safe");
    }

    #[test]
    fn test_allow_list_includes() {
        let filter = McpPolicyFilter::with_lists(vec!["allowed".to_string()], Vec::new());
        let configs = vec![
            stdio_config("allowed", "cmd-ok"),
            stdio_config("other", "cmd-other"),
        ];
        let result = filter.filter(configs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "allowed");
    }

    #[test]
    fn test_deny_overrides_allow() {
        let filter =
            McpPolicyFilter::with_lists(vec!["both".to_string()], vec!["both".to_string()]);
        let config = stdio_config("both", "cmd");
        assert!(!filter.is_allowed(&config));
    }

    #[test]
    fn test_url_matching() {
        let filter = McpPolicyFilter::with_lists(Vec::new(), vec!["http://evil.com/*".to_string()]);
        let good = http_config("good", "http://safe.com/mcp");
        let bad = http_config("bad", "http://evil.com/mcp");
        assert!(filter.is_allowed(&good));
        assert!(!filter.is_allowed(&bad));
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("prefix*", "prefix-something"));
        assert!(glob_match("*suffix", "something-suffix"));
        assert!(glob_match("a*b*c", "aXXbXXc"));
        assert!(!glob_match("exact", "no"));
        assert!(glob_match("exact", "exact"));
    }
}
