use std::collections::HashMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::action::{ToolCallRequest, ToolDefinition, ToolExecutionResult};
use crate::tools::list_dir::ListDirTool;
use crate::tools::read_file::ReadFileTool;
use crate::tools::shell::ShellTool;
use crate::tools::{DynTool, Tool};

#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, DynTool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn with_v1_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(ShellTool::default()));
        registry.register(Arc::new(ReadFileTool::default()));
        registry.register(Arc::new(ListDirTool::default()));
        registry
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.definition().name;
        self.tools.insert(name, tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }

    pub async fn execute(
        &self,
        call: &ToolCallRequest,
        cancel: CancellationToken,
    ) -> ToolExecutionResult {
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

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_v1_defaults()
    }
}
