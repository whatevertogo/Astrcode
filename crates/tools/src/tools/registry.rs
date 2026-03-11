use std::collections::HashMap;
use std::sync::Arc;

use astrcode_core::ToolContext;

use astrcode_core::{ToolCallRequest, ToolDefinition, ToolExecutionResult};
use crate::tools::edit_file::EditFileTool;
use crate::tools::find_files::FindFilesTool;
use crate::tools::grep::GrepTool;
use crate::tools::list_dir::ListDirTool;
use crate::tools::read_file::ReadFileTool;
use crate::tools::shell::ShellTool;
use crate::tools::write_file::WriteFileTool;
use crate::tools::{DynTool, Tool};

#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, DynTool>,
    order: Vec<String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn with_v1_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(ShellTool::default()));
        registry.register(Arc::new(ReadFileTool::default()));
        registry.register(Arc::new(ListDirTool::default()));
        registry.register(Arc::new(WriteFileTool::default()));
        registry.register(Arc::new(EditFileTool::default()));
        registry.register(Arc::new(GrepTool::default()));
        registry.register(Arc::new(FindFilesTool::default()));
        registry
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let definition = tool.definition();
        let name = definition.name;

        if let Some(index) = self.order.iter().position(|existing| existing == &name) {
            self.order.remove(index);
        }
        self.order.push(name.clone());
        self.tools.insert(name, tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.order
            .iter()
            .filter_map(|name| self.tools.get(name))
            .map(|tool| tool.definition())
            .collect()
    }

    pub fn names(&self) -> Vec<String> {
        self.order.clone()
    }

    pub async fn execute(
        &self,
        call: &ToolCallRequest,
        cancel: CancellationToken,
    ) -> ToolExecutionResult {
        if call.name == "shell" {
            if let Err(reason) = validate_shell_command_policy(&call.args) {
                return ToolExecutionResult {
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    ok: false,
                    output: String::new(),
                    error: Some(reason.to_string()),
                    metadata: None,
                    duration_ms: 0,
                };
            }
        }

        let Some(tool) = self.tools.get(&call.name) else {
            return ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("unknown tool '{}'", call.name)),
                metadata: None,
                duration_ms: 0,
            };
        };

        match tool
            .execute(call.id.clone(), call.args.clone(), cancel)
            .await
        {
            Ok(result) => result,
            Err(error) => ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(error.to_string()),
                metadata: None,
                duration_ms: 0,
            },
        }
    }
}

fn validate_shell_command_policy(args: &serde_json::Value) -> anyhow::Result<()> {
    if std::env::var("ASTRCODE_ALLOW_DANGEROUS_SHELL")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return Ok(());
    }

    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_default();

    if command.trim().is_empty() {
        anyhow::bail!("shell command is required");
    }

    const DENY_PATTERNS: [&str; 14] = [
        "rm -rf",
        "rd /s /q",
        "del /f",
        "format ",
        "shutdown",
        "reboot",
        "mkfs",
        "diskpart",
        "reg delete",
        "remove-item -recurse -force",
        "invoke-expression",
        "iex ",
        "curl | sh",
        "wget | sh",
    ];

    if DENY_PATTERNS
        .iter()
        .any(|pattern| command.contains(pattern))
    {
        anyhow::bail!("shell command blocked by policy");
    }

    Ok(())
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_v1_defaults()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::json;
    use astrcode_core::ToolContext;

    use astrcode_core::{ToolCallRequest, ToolDefinition, ToolExecutionResult};
    use astrcode_core::Tool;

    use super::ToolRegistry;

    struct FakeShellTool;

    #[async_trait]
    impl Tool for FakeShellTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "shell".to_string(),
                description: "fake shell for testing".to_string(),
                parameters: json!({"type": "object"}),
            }
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _args: serde_json::Value,
            _cancel: CancellationToken,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "shell".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
            })
        }
    }

    #[tokio::test]
    async fn execute_blocks_dangerous_shell_command() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FakeShellTool));

        let result = registry
            .execute(
                &ToolCallRequest {
                    id: "tc-danger".to_string(),
                    name: "shell".to_string(),
                    args: json!({ "command": "rm -rf /tmp/foo" }),
                },
                test_tool_context(),
            )
            .await;

        assert!(!result.ok);
        assert_eq!(
            result.error.as_deref(),
            Some("shell command blocked by policy")
        );
    }

    #[tokio::test]
    async fn execute_allows_safe_shell_command() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FakeShellTool));

        let result = registry
            .execute(
                &ToolCallRequest {
                    id: "tc-safe".to_string(),
                    name: "shell".to_string(),
                    args: json!({ "command": "echo ok" }),
                },
                test_tool_context(),
            )
            .await;

        assert!(result.ok);
        assert_eq!(result.output, "ok");
    }

    struct NamedTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for NamedTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.to_string(),
                description: format!("{} tool", self.name),
                parameters: json!({"type": "object"}),
            }
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _args: serde_json::Value,
            _cancel: CancellationToken,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: self.name.to_string(),
                ok: true,
                output: String::new(),
                error: None,
                metadata: None,
                duration_ms: 0,
            })
        }
    }

    #[test]
    fn names_returns_registration_order() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(NamedTool { name: "first" }));
        registry.register(Arc::new(NamedTool { name: "second" }));
        registry.register(Arc::new(NamedTool { name: "third" }));

        assert_eq!(
            registry.names(),
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
    }

    #[test]
    fn definitions_return_registration_order() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(NamedTool { name: "first" }));
        registry.register(Arc::new(NamedTool { name: "second" }));
        registry.register(Arc::new(NamedTool { name: "third" }));

        let names = registry
            .definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ]
        );
    }
}
