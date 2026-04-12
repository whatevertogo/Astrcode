use std::sync::Arc;

use astrcode_core::{
    Result, SpawnAgentParams, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::agent_tools::{
    executor::SubAgentExecutor,
    result_mapping::{invalid_params_result, map_subrun_result},
};

const TOOL_NAME: &str = "spawn";

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
        r#"Spawn a dedicated sub-agent to run a specific task and return a summary result.

## Usage Guide

1. **Choose the right profile**: Set `type` to the target profile identifier. Available profiles are listed in the current session's agent index.
2. **Be specific in `prompt`**: Clearly state what to do, find, or analyze.
3. **Add context if needed**: Use `context` for background information (e.g., "focus on security issues", "frontend directory only").
4. **Async by default**: `spawn` launches a background sub-session; progress is streamed back via the session event channel.
5. **Preserve the original agentId**: Copy the `agentId` from the tool result byte-for-byte into later `send` / `observe` / `close` calls — never zero-pad, rewrite, or guess.
6. **Parallel execution**: Issue multiple `spawn` calls in the same turn to run tasks in parallel.
7. **Chained execution**: Wait for each agent's work, read the `summary`, then pass it explicitly in the next step's `context`.

## When to Use

- Exploring a large codebase or finding specific code patterns
- Creating detailed implementation plans
- Multi-perspective code review
- Targeted code modification tasks

## When NOT to Use

- Simple file reads or searches (use `readFile`, `grep` etc. directly)
- Questions you already know the answer to
- Simple operations that don't need an isolated context"#
            .to_string()
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "type": {
                    "type": "string",
                    "description": "Agent profile name. Leave empty for default 'explore'. Available profiles are listed in the session's agent index."
                },
                "description": {
                    "type": "string",
                    "description": "3-5 word short summary for UI/logs only. Not used as task instruction."
                },
                "prompt": {
                    "type": "string",
                    "description": "The main task instruction for the sub-agent. Must be specific and clear."
                },
                "context": {
                    "type": "string",
                    "description": "Optional supplement. E.g. 'focus on security issues', 'frontend directory only'."
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
            // spawn 统一为后台启动：工具本身只负责建链和返回句柄，
            // 不会阻塞当前 agent 的 LLM 轮次，因此可以安全并发。
            .concurrency_safe(true)
            // compact 模式下可以折叠 spawn 的 tool result，减少上下文占用
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "Launch a sub-agent with an isolated context. Only use when parallel benefit, \
                     context isolation, or responsibility separation is clear.",
                    "Use `spawn` to delegate exploration, review, planning, or targeted modification \
                     to a sub-agent. Prefer spawn when: the task would consume significant context, \
                     benefits from parallel execution, or needs an independent responsibility \
                     boundary. Do not delegate simple reads, one-off searches, or operations you \
                     can complete immediately. After calling, remember the original `agentId` from \
                     the tool result; all subsequent `send`, `observe`, `close` calls must reuse \
                     it byte-for-byte.",
                )
                .caveat(
                    "If your next step depends on the result, doing it yourself is usually faster; \
                     only spawn when parallel or isolation value is clear.",
                )
                .caveat(
                    "`description` is for UI/log summary only — put real task requirements in \
                     `prompt`. Choose the narrowest profile for `type`; omit it to use the default \
                     `explore`.",
                )
                .example(
                    "Parallel exploration: { description: \"check cache layer\", prompt: \"review \
                     concurrency and invalidation risks in crates/runtime-cache\", type: \
                     \"reviewer\" }",
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
        let params = match serde_json::from_value::<SpawnAgentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(invalid_params_result(
                    tool_call_id,
                    format!("invalid spawn params: {error}"),
                ));
            },
        };

        // 参数校验在工具层尽早完成，避免把无意义请求下沉到 runtime。
        if let Err(err) = params.validate() {
            return Ok(invalid_params_result(
                tool_call_id,
                format!("invalid spawn params: {err}"),
            ));
        }

        // 将 tool_call_id 注入 context，runtime 层据此关联子会话与发起者
        let launch_ctx = ctx.clone().with_tool_call_id(tool_call_id.clone());
        let result = self.launcher.launch(params, &launch_ctx).await?;
        // 结果映射会统一注入 childRef/openSessionId 等稳定元数据，
        // 让后续 send/observe/close 可以直接复用同一 identity
        Ok(map_subrun_result(tool_call_id, result))
    }
}
