use std::sync::Arc;

use astrcode_core::{
    Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult,
    ToolPromptMetadata, WaitAgentParams,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "waitAgent";

/// 等待指定 child agent 的协作工具。
pub struct WaitAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl WaitAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"等待指定子 Agent 到达下一个可消费状态。

## 使用指南

1. **指定 agentId**: 填入目标子 Agent ID
2. **精确复用 ID**: `agentId` 必须逐字复制自之前 tool result 的 `Child agent reference`，不能补零、改写或猜测
3. **等待条件**: 默认 `final` 等待终态；设为 `next_delivery` 可等待下一次交付

## 何时使用

- 需要阻塞等待子 Agent 完成后再继续
- 需要在子 Agent 产出阶段性交付后立刻处理
- 不要对无关子 Agent 使用，避免不必要的阻塞"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "目标子 Agent 稳定 ID。"
                },
                "until": {
                    "type": "string",
                    "enum": ["final", "next_delivery"],
                    "description": "等待条件：final 等待终态，next_delivery 等待下一次交付。默认 final。"
                }
            },
            "required": ["agentId"]
        })
    }
}

#[async_trait]
impl Tool for WaitAgentTool {
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
            // wait 会阻塞当前 agent 执行，不能并发调用
            .concurrency_safe(false)
            .compact_clearable(false)
            .prompt(
                ToolPromptMetadata::new(
                    "等待子 Agent 到达可消费状态",
                    "使用 waitAgent 阻塞等待子 Agent 完成（final）或产出下一次交付（next_delivery）。\
                    默认等待终态，适合需要子 Agent 完整结果后再继续的场景。`agentId` 必须来自\
                    前一个协作 tool result 的 `Child agent reference`，并逐字复用。",
                )
                .caveat("只等待指定的子 Agent，不影响其他子 Agent；不要把 `agent-1` 改写成 `agent-01`")
                .prompt_tag("collaboration"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let params = match serde_json::from_value::<WaitAgentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(collaboration_error_result(
                    tool_call_id,
                    TOOL_NAME,
                    format!("invalid waitAgent params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid waitAgent params: {err}"),
            ));
        }

        let result = self.executor.wait(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
