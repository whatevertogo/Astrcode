use std::sync::Arc;

use astrcode_core::{
    Result, SendAgentParams, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::agent_tools::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "send";

/// 向既有 child agent 追加消息的协作工具。
///
/// 消息进入 child agent 的 inbox，由其下一轮 LLM 调用消费。
/// 必须指定 `agentId`，该 ID 来自 spawn 返回结果中的稳定引用。
pub struct SendAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl SendAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"Send a follow-up message or rework request to an existing sub-agent.

Use `send` to continue the same child with one concrete next step.

- Use the exact `agentId` returned earlier
- Send one clear instruction, revision request, or narrowed follow-up
- Add `context` only when it materially changes the task

Do not use `send` for status checks, vague reminders, or replacing a reusable child with a new spawn."#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "Target sub-agent stable ID."
                },
                "message": {
                    "type": "string",
                    "description": "Message content to send to the sub-agent."
                },
                "context": {
                    "type": "string",
                    "description": "Optional supplementary context."
                }
            },
            "required": ["agentId", "message"]
        })
    }
}

#[async_trait]
impl Tool for SendAgentTool {
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
            .concurrency_safe(true)
            // send 的 tool result 不应在 compact 模式下被折叠，
            // 因为它包含 childRef，LLM 需要逐字复用其中的 agentId
            .compact_clearable(false)
            .prompt(
                ToolPromptMetadata::new(
                    "Send the next concrete instruction to an existing sub-agent.",
                    "Use `send` when the same child should continue. Write one concrete next \
                     step or revision request, use the exact `agentId`, and prefer `send` over \
                     spawning a new child when the responsibility stays the same.",
                )
                .caveat(
                    "Only send to sub-agents you spawned yourself. Never rewrite `agent-1` as \
                     `agent-01`.",
                )
                .caveat(
                    "Do not use `send` for status checks. If you already know the child is still \
                     running and are simply waiting, do not call `observe` repeatedly either; wait \
                     briefly with your current shell tool instead.",
                )
                .caveat(
                    "Messages enter the child's mailbox and are processed in order. Do not stack \
                     many speculative sends; wait for a result or observe before changing course.",
                )
                .caveat(
                    "Keep the message delta-oriented. Do not restate the whole original brief \
                     when the child already owns the responsibility.",
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
        let params = match serde_json::from_value::<SendAgentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(collaboration_error_result(
                    tool_call_id,
                    TOOL_NAME,
                    format!("invalid send params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid send params: {err}"),
            ));
        }

        let result = self.executor.send(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
