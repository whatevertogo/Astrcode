//! # observe 工具
//!
//! 四工具模型中的观测工具。返回目标 child agent 的增强快照，
//! 融合 live control state、对话投影和 mailbox 派生信息。

use std::sync::Arc;

use astrcode_core::{
    ObserveParams, Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::agent_tools::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "observe";

/// 获取目标 child agent 增强快照的观测工具。
///
/// 只返回直接子 agent 的快照，非直接父、兄弟、跨树调用被拒绝。
/// 快照融合三层数据：live lifecycle、对话投影、mailbox 派生摘要。
pub struct ObserveAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl ObserveAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"Get the current state snapshot of a specified sub-agent.

Use `observe` to decide the next action for one direct child.

- Use the exact `agentId` returned earlier
- Call it only when you cannot decide between `wait`, `send`, or `close` without a fresh snapshot
- Read both the raw facts and the advisory fields in the result
- Treat the mailbox section as a short tail view of recent messages, not as full history

Do not poll repeatedly with no decision attached. If you are simply waiting for a running child,
pause briefly with your current shell tool (for example `sleep`) instead of spending another
tool call on `observe`. Do not use it for unrelated agents."#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "Stable ID of the sub-agent to observe."
                }
            },
            "required": ["agentId"]
        })
    }
}

#[async_trait]
impl Tool for ObserveAgentTool {
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
            // observe 是只读查询，可以安全并发
            .concurrency_safe(true)
            // observe 的 tool result 在 compact 模式下可折叠
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "Observe child state when you need to decide the next action.",
                    "Use `observe` when the next decision depends on current child state. It is \
                     a synchronous query for one direct child and should usually answer `wait`, \
                     `send`, or `close`, not act as a polling loop.",
                )
                .caveat(
                    "Only returns snapshots for direct child agents. Never rewrite `agent-1` as \
                     `agent-01`.",
                )
                .caveat(
                    "`observe` returns raw lifecycle/outcome facts plus advisory decision fields. \
                     Treat the advice as guidance, not as a replacement for the facts.",
                )
                .caveat(
                    "Prefer one well-timed observe over repeated checking. If you are just \
                     waiting for a running child, use your current shell tool to sleep briefly and \
                     then continue, instead of polling `observe` again.",
                )
                .caveat(
                    "`observe` only exposes a short mailbox tail and latest output excerpt. It is \
                     intentionally not a full mailbox or transcript dump.",
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
        let params = match serde_json::from_value::<ObserveParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(collaboration_error_result(
                    tool_call_id,
                    TOOL_NAME,
                    format!("invalid observe params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid observe params: {err}"),
            ));
        }

        let result = self.executor.observe(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
