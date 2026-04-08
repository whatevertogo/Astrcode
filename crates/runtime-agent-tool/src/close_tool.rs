use std::sync::Arc;

use astrcode_core::{
    CloseAgentParams, Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "closeAgent";

/// 关闭指定 child agent 的协作工具。
pub struct CloseAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl CloseAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"关闭指定子 Agent，默认级联关闭其子树。

## 使用指南

1. **指定 agentId**: 填入要关闭的子 Agent ID
2. **精确复用 ID**: `agentId` 必须逐字复制自之前 tool result 的 `Child agent reference`，不能补零、改写或猜测
3. **级联控制**: 默认会级联关闭该 Agent 的所有子 Agent；设 `cascade: false` 仅关闭目标本身
4. **终态检查**: 已终态的 Agent 会被幂等处理，不会报错

## 何时使用

- 子 Agent 的任务已经不再需要
- 需要释放资源给其他 Agent
- 协作完成后主动清理"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "要关闭的子 Agent 稳定 ID。"
                },
                "cascade": {
                    "type": "boolean",
                    "description": "是否级联关闭子树，默认 true。"
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
                    "关闭子 Agent",
                    "使用 closeAgent 关闭不再需要的子 Agent。默认级联关闭子 Agent 的所有子 \
                     Agent。`agentId` 必须来自之前协作 tool result 的 `Child agent \
                     reference`，并逐字复用。",
                )
                .caveat("已终态的子 Agent 会被幂等处理；不要把 `agent-1` 改写成 `agent-01`")
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
                    format!("invalid closeAgent params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid closeAgent params: {err}"),
            ));
        }

        let result = self.executor.close(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
