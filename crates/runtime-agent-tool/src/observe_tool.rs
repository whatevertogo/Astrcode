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

use crate::{
    collab_result_mapping::{collaboration_error_result, map_collaboration_result},
    collaboration_executor::CollaborationExecutor,
};

const TOOL_NAME: &str = "observe";

/// 获取目标 child agent 增强快照的观测工具。
///
/// 只返回直接子 agent 的快照，非直接父、兄弟、跨树调用被拒绝。
/// 快照融合三层数据：live lifecycle、对话投影、mailbox pending count。
pub struct ObserveAgentTool {
    executor: Arc<dyn CollaborationExecutor>,
}

impl ObserveAgentTool {
    pub fn new(executor: Arc<dyn CollaborationExecutor>) -> Self {
        Self { executor }
    }

    fn build_description() -> String {
        r#"获取指定子 Agent 的当前状态快照。

## 使用指南

1. **指定 agentId**: 填入要观测的子 Agent ID
2. **精确复用 ID**: `agentId` 必须逐字复制自之前 tool result 的 `Child agent reference`，不能补零、改写或猜测
3. **快照内容**: 返回包含 lifecycleStatus、lastTurnOutcome、pendingMessageCount 等字段的完整状态快照
4. **决策依据**: 根据快照决定是继续 `send` 新任务还是 `close` 终止

## 返回字段

- `lifecycleStatus`: Pending / Running / Idle / Terminated
- `lastTurnOutcome`: 上一轮执行结果（Completed / Cancelled / Failed / TokenExceeded）
- `pendingMessageCount`: 待处理消息数量（durable replay 为准）
- `turnCount`: 已完成轮次数
- `lastOutput`: 最近输出的摘要

## 何时使用

- 需要了解子 Agent 当前状态再决定下一步
- 检查子 Agent 是否已完成上一轮工作（lifecycleStatus = Idle）
- 查看子 Agent 是否有积压的未处理消息
- 不要对无关子 Agent 使用"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "被观测的子 Agent 稳定 ID。"
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
                    "观测子 Agent 状态",
                    "使用 observe 获取直接子 Agent 的增强状态快照。快照包含 lifecycleStatus、\
                     lastTurnOutcome、pendingMessageCount 等，帮助决定下一步操作。只能观测自己 \
                     spawn 的直接子 Agent。`agentId` 必须来自之前协作 tool result 的 \
                     `Child agent reference`，并逐字复用。",
                )
                .caveat("只返回直接子 Agent 的快照；不要把 `agent-1` 改写成 `agent-01`")
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
