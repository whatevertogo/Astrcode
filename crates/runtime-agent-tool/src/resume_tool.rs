use std::sync::Arc;

use astrcode_core::{
    Result, ResumeAgentParams, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "resumeAgent";

/// 恢复已完成 child agent 的协作工具。
///
/// 恢复操作复用同一 child session（保持上下文连续性），不创建新会话。
/// 仅对终态 agent（completed/failed/cancelled）有效。
pub struct ResumeAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl ResumeAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"恢复一个已完成但仍可继续协作的子 Agent。

## 使用指南

1. **指定 agentId**: 填入要恢复的子 Agent ID
2. **精确复用 ID**: `agentId` 必须逐字复制自之前 tool result 的 `Child agent reference`，不能补零、改写或猜测
3. **追加消息**: 可在 `message` 中说明恢复原因或新需求
4. **复用会话**: 恢复会复用同一 child session，不创建新会话

## 何时使用

- 子 Agent 完成了初始任务，但需要追加返工或修改
- 需要基于上一次的成果继续深入
- 不适用于尚未完成或已取消的 Agent"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "要恢复的子 Agent 稳定 ID。"
                },
                "message": {
                    "type": "string",
                    "description": "恢复后追加给子 Agent 的消息。"
                }
            },
            "required": ["agentId"]
        })
    }
}

#[async_trait]
impl Tool for ResumeAgentTool {
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
            // resume 会重启子 agent 的 LLM 轮次，状态变更期间不应并发操作同一 agent
            .concurrency_safe(false)
            // resume 的 tool result 可折叠，但 agentId 仍需保留供后续协作使用
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "恢复已完成的子 Agent",
                    "使用 resumeAgent 恢复已完成的子 Agent 继续协作。恢复复用同一 child \
                     session，不创建新会话。适合需要追加返工或修改的场景。`agentId` \
                     必须来自之前协作 tool result 的 `Child agent reference`，并逐字复用。",
                )
                .caveat(
                    "只能恢复已完成、已失败或已取消的子 Agent；不要把 `agent-1` 改写成 `agent-01`",
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
        let params = match serde_json::from_value::<ResumeAgentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(collaboration_error_result(
                    tool_call_id,
                    TOOL_NAME,
                    format!("invalid resumeAgent params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid resumeAgent params: {err}"),
            ));
        }

        let result = self.executor.resume(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
