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

## Usage Guide

1. **Specify agentId**: The sub-agent ID to observe.
2. **Copy agentId exactly**: `agentId` must be copied byte-for-byte from a previous tool result's `Child agent reference` — never zero-pad, rewrite, or guess.
3. **Snapshot contents**: Returns a full state snapshot including `lifecycleStatus`, `lastTurnOutcome`, `activeTask`, `pendingTask`, `pendingMessageCount`, etc.
4. **Decision basis**: Use the snapshot to decide whether to `send` a new task or `close` the agent.
5. **Recommended practice**: Use a shell script to wait for a short interval before observing again, to avoid aggressive polling.

## Returned Fields

- `lifecycleStatus`: Pending / Running / Idle / Terminated
- `lastTurnOutcome`: Previous turn result (Completed / Cancelled / Failed / TokenExceeded)
- `activeTask`: Summary of the currently processing task (if any)
- `pendingTask`: Summary of the next pending task (if any)
- `pendingMessageCount`: Number of pending messages (per durable replay)
- `turnCount`: Number of completed turns
- `lastOutput`: Summary of the most recent output

## When to Use

- Need to know the sub-agent's current state before deciding next steps
- Check if a sub-agent has finished its last turn (lifecycleStatus = Idle)
- See if a sub-agent has a backlog of unprocessed messages
- Do NOT use on unrelated sub-agents"#
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
                    "Observe sub-agent state.",
                    "Use `observe` to get an enhanced state snapshot of a direct child agent. The \
                     snapshot includes `lifecycleStatus`, `lastTurnOutcome`, `activeTask`, \
                     `pendingTask`, `pendingMessageCount`, etc. to help decide the next action. \
                     Only direct children spawned by you can be observed. The `agentId` must come \
                     from a previous collaboration tool result's `Child agent reference` and be \
                     reused byte-for-byte.",
                )
                .caveat(
                    "Only returns snapshots for direct child agents. Never rewrite `agent-1` as \
                     `agent-01`.",
                )
                .caveat(
                    "State transitions: Pending → Running → Idle (waiting for new tasks) or \
                     Terminated (closed). `observe` is a synchronous non-blocking query — call at \
                     intervals, do not poll aggressively.",
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
