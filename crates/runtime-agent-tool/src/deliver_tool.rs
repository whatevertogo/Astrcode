use std::sync::Arc;

use astrcode_core::{
    DeliverToParentParams, Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "deliverToParent";

/// 向直接父 agent 交付结果的协作工具。
///
/// 仅 child session 可见，用于把阶段性结果或最终交付送回直接父 agent。
pub struct DeliverToParentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl DeliverToParentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"将当前子 Agent 的结果交付给直接父 Agent。

## 使用指南

1. **填写 summary**: 必须提供一段摘要，说明交付了什么
2. **补充 findings**: 列出关键发现，便于父 Agent 快速了解
3. **最终回复**: 如果这是最终交付，在 `finalReply` 中写完整回复
4. **产物引用**: 如有文件/代码变更，在 `artifacts` 中列出

## 何时使用

- 子 Agent 完成了分配的任务，需要将结果送回父 Agent
- 子 Agent 阶段性完成，需要向父 Agent 汇报进展
- 不要用此工具与其他子 Agent 通信，只能交付给直接父 Agent"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "交付摘要，必须具体明确。"
                },
                "findings": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "关键发现列表。"
                },
                "finalReply": {
                    "type": "string",
                    "description": "最终回复内容，仅终态交付时使用。"
                },
                "artifacts": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string" },
                            "id": { "type": "string" },
                            "label": { "type": "string" }
                        }
                    },
                    "description": "产物引用列表。"
                }
            },
            "required": ["summary"]
        })
    }
}

#[async_trait]
impl Tool for DeliverToParentTool {
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
                    "向父 Agent 交付结果",
                    "使用 deliverToParent 将子 Agent 的结果交付给直接父 Agent。只能交付给直接父 \
                     Agent，不能跨级交付。",
                )
                .caveat("仅子 Agent 可调用此工具")
                .caveat("只能交付给直接父 Agent")
                .prompt_tag("collaboration"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let params = match serde_json::from_value::<DeliverToParentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(collaboration_error_result(
                    tool_call_id,
                    TOOL_NAME,
                    format!("invalid deliverToParent params: {error}"),
                ));
            },
        };

        if let Err(err) = params.validate() {
            return Ok(collaboration_error_result(
                tool_call_id,
                TOOL_NAME,
                format!("invalid deliverToParent params: {err}"),
            ));
        }

        let result = self.executor.deliver(params, ctx).await?;
        Ok(map_collaboration_result(tool_call_id, TOOL_NAME, result))
    }
}
