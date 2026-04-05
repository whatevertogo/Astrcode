//! # Agent as Tool
//!
//! 提供 `runAgent` 工具的稳定抽象：
//! - 对 LLM 暴露统一的工具定义和参数 schema
//! - 将真实执行委托给运行时注入的 `SubAgentExecutor`
//! - 不直接依赖 `RuntimeService`，避免把 runtime 细节扩散到 Tool crate

use std::sync::Arc;

use astrcode_core::{
    Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// `runAgent` 工具的调用参数。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunAgentParams {
    /// 目标 Agent Profile 标识。
    pub name: String,
    /// 子 Agent 要执行的任务描述。
    pub task: String,
    /// 附加上下文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// 可选步数覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
}

/// 子 Agent 执行结果类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentOutcome {
    Completed,
    Failed { error: String },
    Aborted,
    TokenExceeded,
}

impl SubAgentOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Aborted => "aborted",
            Self::TokenExceeded => "token_exceeded",
        }
    }
}

/// 子 Agent 执行的最小返回面。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentResult {
    pub outcome: SubAgentOutcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// 子 Agent 执行器抽象。
///
/// 真实执行器由 runtime 提供，这里只定义 Tool 所需的最小边界。
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    async fn execute(&self, params: RunAgentParams, ctx: &ToolContext) -> Result<SubAgentResult>;
}

/// 把子 Agent 能力暴露给 LLM 的内置工具。
pub struct RunAgentTool {
    executor: Arc<dyn SubAgentExecutor>,
}

impl RunAgentTool {
    pub fn new(executor: Arc<dyn SubAgentExecutor>) -> Self {
        Self { executor }
    }

    fn parameters_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {
                    "type": "string",
                    "description": "要调用的 agent profile 标识"
                },
                "task": {
                    "type": "string",
                    "description": "子 Agent 需要完成的具体任务"
                },
                "context": {
                    "type": "string",
                    "description": "可选的补充上下文"
                },
                "maxSteps": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "可选的 step 上限覆盖"
                }
            },
            "required": ["name", "task"]
        })
    }

    fn invalid_params_result(tool_call_id: String, message: String) -> ToolExecutionResult {
        ToolExecutionResult {
            tool_call_id,
            tool_name: "runAgent".to_string(),
            ok: false,
            output: String::new(),
            error: Some(message),
            metadata: None,
            duration_ms: 0,
            truncated: false,
        }
    }
}

#[async_trait]
impl Tool for RunAgentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "runAgent".to_string(),
            description: "调用受限子 Agent 执行一个专门任务，并返回摘要结果。".to_string(),
            parameters: Self::parameters_schema(),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tag("agent")
            .tag("subagent")
            .compact_clearable(true)
    }

    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let params = match serde_json::from_value::<RunAgentParams>(input) {
            Ok(params) => params,
            Err(error) => {
                return Ok(Self::invalid_params_result(
                    tool_call_id,
                    format!("invalid runAgent params: {error}"),
                ));
            },
        };

        let result = self.executor.execute(params, ctx).await?;
        let mut metadata = result.metadata.unwrap_or_else(|| json!({}));
        if let Value::Object(object) = &mut metadata {
            object.insert(
                "outcome".to_string(),
                Value::String(result.outcome.as_str().to_string()),
            );
        }

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "runAgent".to_string(),
            ok: !matches!(result.outcome, SubAgentOutcome::Failed { .. }),
            output: result.summary,
            error: match result.outcome {
                SubAgentOutcome::Failed { error } => Some(error),
                _ => None,
            },
            metadata: Some(metadata),
            duration_ms: 0,
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{CancelToken, Tool, ToolContext};
    use async_trait::async_trait;
    use serde_json::json;

    use super::{RunAgentParams, RunAgentTool, SubAgentExecutor, SubAgentOutcome, SubAgentResult};

    struct RecordingExecutor {
        calls: Mutex<Vec<RunAgentParams>>,
    }

    #[async_trait]
    impl SubAgentExecutor for RecordingExecutor {
        async fn execute(
            &self,
            params: RunAgentParams,
            _ctx: &ToolContext,
        ) -> astrcode_core::Result<SubAgentResult> {
            self.calls.lock().expect("calls lock").push(params);
            Ok(SubAgentResult {
                outcome: SubAgentOutcome::Completed,
                summary: "done".to_string(),
                metadata: Some(json!({"agentId": "agent-1"})),
            })
        }
    }

    fn tool_context() -> ToolContext {
        ToolContext::new(
            "session-1".to_string(),
            std::env::temp_dir(),
            CancelToken::new(),
        )
    }

    #[tokio::test]
    async fn run_agent_tool_parses_params_and_returns_summary() {
        let executor = Arc::new(RecordingExecutor {
            calls: Mutex::new(Vec::new()),
        });
        let tool = RunAgentTool::new(executor.clone());

        let result = tool
            .execute(
                "call-1".to_string(),
                json!({
                    "name": "review",
                    "task": "inspect changes",
                    "context": "focus on tests",
                    "maxSteps": 3
                }),
                &tool_context(),
            )
            .await
            .expect("tool execution should succeed");

        assert!(result.ok);
        assert_eq!(result.output, "done");
        let calls = executor.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "review");
        assert_eq!(calls[0].max_steps, Some(3));
    }

    #[tokio::test]
    async fn run_agent_tool_reports_invalid_params_as_tool_failure() {
        let tool = RunAgentTool::new(Arc::new(RecordingExecutor {
            calls: Mutex::new(Vec::new()),
        }));

        let result = tool
            .execute(
                "call-2".to_string(),
                json!({"name": "review"}),
                &tool_context(),
            )
            .await
            .expect("tool should convert validation failure into tool result");

        assert!(!result.ok);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("invalid runAgent params"))
        );
    }
}
