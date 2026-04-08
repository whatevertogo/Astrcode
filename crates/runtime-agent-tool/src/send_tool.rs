use std::sync::Arc;

use astrcode_core::{
    Result, SendAgentParams, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "sendAgent";

/// 向既有 child agent 追加消息的协作工具。
pub struct SendAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl SendAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"向既有子 Agent 追加要求或返工请求。

## 使用指南

1. **指定 agentId**: 填入目标子 Agent ID
2. **填写 message**: 追加给子 Agent 的消息内容
3. **补充 context**: 可选的补充上下文信息

## 何时使用

- 需要向正在运行的子 Agent 追加信息或修改要求
- 子 Agent 完成后需要返工或补充
- 不要用于创建新 Agent（用 `spawnAgent`）"#
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
                "message": {
                    "type": "string",
                    "description": "追加给子 Agent 的消息内容。"
                },
                "context": {
                    "type": "string",
                    "description": "可选补充上下文。"
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
            .compact_clearable(false)
            .prompt(
                ToolPromptMetadata::new(
                    "向已有子 Agent 追加消息",
                    "使用 sendAgent 向正在运行或已完成的子 Agent 追加要求或返工请求。目标通过稳定 \
                     agentId 指定，该 ID 来自 spawnAgent 的返回结果。",
                )
                .caveat("只能向自己 spawn 的子 Agent 发送消息")
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
                    format!("invalid sendAgent params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid sendAgent params: {err}"),
            ));
        }

        let result = self.executor.send(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
