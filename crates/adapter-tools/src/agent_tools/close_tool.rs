use std::sync::Arc;

use astrcode_core::{
    CloseAgentParams, Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::agent_tools::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "close";

/// 关闭指定 child agent 及其子树的协作工具。
///
/// 始终级联关闭整棵子树，因为孤立子 agent 无法自行终止。
pub struct CloseAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl CloseAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"Close a sub-agent and cascade-close its subtree.

## Usage Guide

1. **Specify agentId**: The sub-agent ID to close.
2. **Copy agentId exactly**: `agentId` must be copied byte-for-byte from a previous tool result's `Child agent reference`.
3. **Cascade close**: All descendants of the agent are closed together.

## When to Use

- The child has finished and you no longer need follow-up work
- The child took a wrong direction and you want to stop that branch
- You want to free capacity before spawning other children

## When NOT to Use

- You still expect more work from the same child; use `send`
- You only want to inspect state; use `observe`
- You are unsure whether the task is complete"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "Stable ID of the sub-agent to close."
                }
            },
            "required": ["agentId"]
        })
    }
}

#[async_trait]
impl Tool for CloseAgentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: TOOL_NAME.to_string(),
            description: Self::build_description(),
            parameters: Self::parameters_schema(),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tag("agent")
            .tag("collaboration")
            .concurrency_safe(false)
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "Close a child branch that is finished or no longer useful.",
                    "Use `close` when a child has completed its job, when you want to stop an \
                     unneeded branch, or when you need to free capacity. Closing cascades through \
                     the child's subtree. Reuse the exact `agentId` returned earlier.",
                )
                .caveat(
                    "Already-terminated sub-agents are handled idempotently. Never rewrite \
                     `agent-1` as `agent-01`.",
                )
                .caveat(
                    "Closing cascades to all descendant agents. After `close`, do not call `send` \
                     or `observe` on that agentId.",
                )
                .caveat(
                    "When unsure whether the child is still needed, `observe` first. Use `close` \
                     for cleanup, not as a status probe.",
                )
                .prompt_tag("collaboration"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let params = match serde_json::from_value::<CloseAgentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(collaboration_error_result(
                    tool_call_id,
                    TOOL_NAME,
                    format!("invalid close params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid close params: {err}"),
            ));
        }

        let result = self.executor.close(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
