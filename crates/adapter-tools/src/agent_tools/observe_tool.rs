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
2. **Copy agentId exactly**: `agentId` must be copied byte-for-byte from a previous tool result's `Child agent reference`.
3. **Use it to decide an action**: `observe` should usually answer "send next", "wait", or "close".
4. **Do not poll aggressively**: Observe when a decision depends on the state, not in a tight loop.

## Returned Fields

- `lifecycleStatus`: Pending / Running / Idle / Terminated
- `lastTurnOutcome`: Previous turn result (Completed / Cancelled / Failed / TokenExceeded)
- `activeTask`: Summary of the currently processing task (if any)
- `pendingTask`: Summary of the next pending task (if any)
- `pendingMessageCount`: Number of pending messages (per durable replay)
- `turnCount`: Number of completed turns
- `lastOutput`: Summary of the most recent output

## When to Use

- Need to know whether a child is still running, idle, or done
- Need the latest outcome before deciding whether to `send` or `close`
- Need to confirm whether the child has pending mailbox work

## When NOT to Use

- Repeated heartbeat polling with no decision attached
- Reading output that you already received in a delivery
- Observing unrelated agents"#
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
                    "Use `observe` to answer a concrete question such as: Is this child still \
                     running? Did it finish its last turn? Should I `send` another instruction or \
                     `close` it? Observe only direct children you spawned, and reuse the exact \
                     `agentId` returned earlier.",
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
                .caveat(
                    "Prefer one well-timed observe over repeated checking. If a child just sent a \
                     usable delivery, act on it instead of observing immediately again.",
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
