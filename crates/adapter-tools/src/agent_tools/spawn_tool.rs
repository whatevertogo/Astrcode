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
2. **Be specific in `prompt`**: Give one concrete responsibility, deliverable, or review angle.
3. **Add only useful context**: Use `context` for narrow constraints such as scope, quality bar, or files to focus on.
4. **Background by default**: `spawn` starts a child and returns immediately.
5. **Preserve the original agentId**: Copy the returned `agentId` exactly into later `send` / `observe` / `close` calls.
6. **Start with one child**: Add more children only when work clearly splits into independent threads.
7. **Reuse before respawn**: If a child is `Idle`, prefer `send` or `observe` over creating another child for the same responsibility.
8. **Depth and fan-out are limited**: Reuse an existing child before creating a deeper or wider subtree.

## When to Use

- Exploring a large codebase or finding specific code patterns
- Creating detailed implementation plans
- Multi-perspective code review
- Targeted code modification tasks with clear ownership boundaries

## When NOT to Use

- Simple file reads or searches (use `readFile`, `grep` etc. directly)
- Questions you already know the answer to
- Broad "explore everything" delegation without first defining a few concrete workstreams"#
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
            // spawn 会修改 agent 树与 child session 目录视图；
            // 串行执行可以避免同轮 fan-out 检查发生竞态。
            .concurrency_safe(false)
            // compact 模式下可以折叠 spawn 的 tool result，减少上下文占用
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "Launch a sub-agent with an isolated context. Only use when parallel benefit, \
                     context isolation, or responsibility separation is clear.",
                    "Use `spawn` to delegate exploration, review, planning, or targeted modification \
                     to a sub-agent when there is clear isolation or parallel value. Start with one \
                     child unless there are clearly separate workstreams. Give the child one narrow \
                     responsibility, not a vague request to explore everything. Do not delegate \
                     simple reads, one-off searches, or work you can finish immediately. After \
                     calling, reuse the returned `agentId` exactly in later `send`, `observe`, and \
                     `close` calls. Reuse an idle child before spawning another child for the same \
                     thread of work.",
                )
                .caveat(
                    "If your next step depends on the result, doing it yourself is usually faster; \
                     only spawn when parallel or isolation value is clear.",
                )
                .caveat(
                    "Do not fan out by default. A small number of well-scoped children is better \
                     than spawning many vague explorers over the same repo surface.",
                )
                .caveat(
                    "`description` is for UI/log summary only — put real task requirements in \
                     `prompt`. Choose the narrowest profile for `type`; omit it to use the default \
                     `explore`.",
                )
                .caveat(
                    "If spawn fails because a depth or fan-out limit is reached, do not keep \
                     retrying with more children. Reuse an existing child via `send`, or finish \
                     the work in the current agent.",
                )
                .example(
                    "Focused delegation: { description: \"check cache layer\", prompt: \"review \
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
