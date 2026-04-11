use std::sync::Arc;

use astrcode_core::{
    Result, SpawnAgentParams, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    executor::SubAgentExecutor,
    result_mapping::{invalid_params_result, map_subrun_result},
};

const TOOL_NAME: &str = "spawnAgent";

/// 把子 Agent 能力暴露给 LLM 的内置工具。
///
/// 持有一个 `SubAgentExecutor` trait object，将实际的 session 创建和 agent 启动
/// 委托给 runtime 层，本工具只负责参数 schema 定义、校验和结果映射。
pub struct SpawnAgentTool {
    launcher: Arc<dyn SubAgentExecutor>,
}

impl SpawnAgentTool {
    pub fn new(launcher: Arc<dyn SubAgentExecutor>) -> Self {
        Self { launcher }
    }

    fn build_description() -> String {
        r#"调用专门的子 Agent 执行特定任务，并返回摘要结果。

## 使用指南

1. **选择合适的 Agent**: `type` 填目标 profile 标识；可用 profile 以当前会话提供的 agent 索引或提示信息为准
2. **写清楚任务**: `prompt` 参数要具体、明确，说明要做什么、找什么、分析什么
3. **补充上下文**: 如果任务涉及特定背景，在 `context` 中说明（如"关注安全问题"、"只看 frontend 目录"）
4. **默认异步**: `spawnAgent` 统一用后台子会话方式启动，通过子会话流持续回传进度
5. **记住原始 agentId**: 后续 `waitAgent` / `sendAgent` / `closeAgent` / `resumeAgent` 必须逐字复用 tool result 里的 `agentId`，不能补零、改写或猜测
6. **并行执行**: 需要并行时，直接在同一轮对话中发起多个 `spawnAgent` 调用即可
7. **链式执行**: 需要链式时，你可以等待每个 agent 的工作，读取前一步的 `summary`，然后在下一步的 `context` 中显式传入

## 何时使用

- 需要探索大型代码库或查找特定代码模式
- 需要制定详细的实现计划
- 需要对代码变更进行多角度审查
- 需要执行定向的代码修改任务

## 何时不使用

- 简单的文件读取或搜索（直接用 `readFile`、`grep` 等工具更快）
- 已经清楚答案的确认性问题
- 不需要独立上下文的简单操作"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "type": {
                    "type": "string",
                    "description": "Agent profile 名称。留空默认 'explore'。可用 profile 以当前会话里的 agent 索引或提示信息为准。"
                },
                "description": {
                    "type": "string",
                    "description": "3-5 词短摘要，仅供 UI/日志展示。不作为任务指令。"
                },
                "prompt": {
                    "type": "string",
                    "description": "要执行的任务正文。这是子 Agent 收到的指令主体，必须具体明确。"
                },
                "context": {
                    "type": "string",
                    "description": "可选补充。如'关注安全问题'、'只看 frontend 目录'。"
                }
            },
            "required": ["description", "prompt"]
        })
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
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
            .tag("subagent")
            // spawnAgent 统一为后台启动：工具本身只负责建链和返回句柄，
            // 不会阻塞当前 agent 的 LLM 轮次，因此可以安全并发。
            .concurrency_safe(true)
            // compact 模式下可以折叠 spawnAgent 的 tool result，减少上下文占用
            .compact_clearable(true)
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let params = match serde_json::from_value::<SpawnAgentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(invalid_params_result(
                    tool_call_id,
                    format!("invalid spawnAgent params: {error}"),
                ));
            },
        };

        // 参数校验在工具层尽早完成，避免把无意义请求下沉到 runtime。
        if let Err(err) = params.validate() {
            return Ok(invalid_params_result(
                tool_call_id,
                format!("invalid spawnAgent params: {err}"),
            ));
        }

        // 将 tool_call_id 注入 context，runtime 层据此关联子会话与发起者
        let launch_ctx = ctx.clone().with_tool_call_id(tool_call_id.clone());
        let result = self.launcher.launch(params, &launch_ctx).await?;
        // 结果映射会统一注入 childRef/openSessionId 等稳定元数据，
        // 让后续 send/wait/resume/close 可以直接复用同一 identity
        Ok(map_subrun_result(tool_call_id, result))
    }
}
