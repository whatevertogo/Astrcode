//! # Agent as Tool
//!
//! 提供 `spawnAgent` 工具的稳定抽象：
//! - 对 LLM 暴露统一的工具定义和参数 schema
//! - 将真实执行委托给运行时注入的 `SubAgentExecutor`
//! - 不直接依赖 `RuntimeService`，避免把 runtime 细节扩散到 Tool crate

use std::sync::Arc;

pub use astrcode_core::SpawnAgentParams;
use astrcode_core::{
    AgentProfile, Result, SubRunOutcome, SubRunResult, Tool, ToolCapabilityMetadata, ToolContext,
    ToolDefinition, ToolExecutionResult,
};
use async_trait::async_trait;
use serde_json::{Value, json};

/// 子 Agent 执行器抽象。
///
/// 真实执行器由 runtime 提供，这里只定义 Tool 所需的最小边界。
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    /// 启动子 Agent。
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult>;
}

/// 子 Agent profile 目录抽象。
///
/// 动态 profile 列表不应该再内嵌进 `spawnAgent` 的静态 tool definition，
/// 这里单独抽出 discovery 边界，供 prompt contributor 或独立索引能力按需使用。
pub trait AgentProfileCatalog: Send + Sync {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile>;
}

/// 把子 Agent 能力暴露给 LLM 的内置工具。
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
5. **并行执行**: 需要并行时，直接在同一轮对话中发起多个 `spawnAgent` 调用即可
6. **链式执行**: 需要链式时，你可以等待每个 agent 的工作，读取前一步的 `summary`，然后在下一步的 `context` 中显式传入

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

    fn invalid_params_result(tool_call_id: String, message: String) -> ToolExecutionResult {
        ToolExecutionResult {
            tool_call_id,
            tool_name: "spawnAgent".to_string(),
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
impl Tool for SpawnAgentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawnAgent".to_string(),
            description: Self::build_description(),
            parameters: Self::parameters_schema(),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tag("agent")
            .tag("subagent")
            // `spawnAgent` 已统一为后台启动，工具本身只负责快速建链和返回句柄，
            // 可以安全地和其他同类启动请求并发执行。
            .concurrency_safe(true)
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
                return Ok(Self::invalid_params_result(
                    tool_call_id,
                    format!("invalid spawnAgent params: {error}"),
                ));
            },
        };

        // 参数校验在工具层尽早完成，避免把无意义请求下沉到 runtime。
        if let Err(err) = params.validate() {
            return Ok(Self::invalid_params_result(
                tool_call_id,
                format!("invalid spawnAgent params: {err}"),
            ));
        }

        let launch_ctx = ctx.clone().with_tool_call_id(tool_call_id.clone());
        let result = self.launcher.launch(params, &launch_ctx).await?;
        let mut metadata = json!({
            "outcome": result.status.as_str(),
            "handoff": result.handoff,
            "failure": result.failure,
            "result": result,
        });
        if let Value::Object(object) = &mut metadata {
            object.insert(
                "schema".to_string(),
                Value::String("subRunResult".to_string()),
            );
        }
        let output = tool_output_for_result(&result);
        let error = result
            .failure
            .as_ref()
            .map(|failure| failure.technical_message.clone());

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "spawnAgent".to_string(),
            ok: !matches!(result.status, SubRunOutcome::Failed),
            output,
            error,
            metadata: Some(metadata),
            duration_ms: 0,
            truncated: false,
        })
    }
}

fn tool_output_for_result(result: &SubRunResult) -> String {
    match result.status {
        SubRunOutcome::Failed => result
            .failure
            .as_ref()
            .map(|failure| failure.display_message.clone())
            .unwrap_or_else(|| "子 Agent 执行失败。".to_string()),
        _ => result
            .handoff
            .as_ref()
            .map(|handoff| handoff.summary.clone())
            .unwrap_or_else(|| "子 Agent 未返回摘要。".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{
        CancelToken, SubRunFailure, SubRunFailureCode, SubRunHandoff, SubRunOutcome, SubRunResult,
        Tool, ToolContext,
    };
    use async_trait::async_trait;
    use serde_json::json;

    use super::{SpawnAgentParams, SpawnAgentTool, SubAgentExecutor};

    struct RecordingExecutor {
        calls: Mutex<Vec<SpawnAgentParams>>,
    }

    #[async_trait]
    impl SubAgentExecutor for RecordingExecutor {
        async fn launch(
            &self,
            params: SpawnAgentParams,
            _ctx: &ToolContext,
        ) -> astrcode_core::Result<SubRunResult> {
            self.calls.lock().expect("calls lock").push(params);
            Ok(SubRunResult {
                status: SubRunOutcome::Completed,
                handoff: Some(SubRunHandoff {
                    summary: "done".to_string(),
                    findings: vec!["checked".to_string()],
                    artifacts: Vec::new(),
                }),
                failure: None,
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
    async fn spawn_agent_tool_parses_params_and_returns_summary() {
        let executor = Arc::new(RecordingExecutor {
            calls: Mutex::new(Vec::new()),
        });
        let tool = SpawnAgentTool::new(executor.clone());

        let result = tool
            .execute(
                "call-1".to_string(),
                json!({
                    "type": "explore",
                    "description": "inspect changes",
                    "prompt": "inspect changes",
                    "context": "focus on tests"
                }),
                &tool_context(),
            )
            .await
            .expect("tool execution should succeed");

        assert!(result.ok);
        assert_eq!(result.output, "done");
        let calls = executor.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].r#type, Some("explore".to_string()));
        assert_eq!(
            result
                .metadata
                .as_ref()
                .and_then(|value| value.get("schema")),
            Some(&json!("subRunResult"))
        );
    }

    #[tokio::test]
    async fn spawn_agent_tool_reports_invalid_params_as_tool_failure() {
        let tool = SpawnAgentTool::new(Arc::new(RecordingExecutor {
            calls: Mutex::new(Vec::new()),
        }));

        let result = tool
            .execute(
                "call-2".to_string(),
                json!({"name": "explore"}),
                &tool_context(),
            )
            .await
            .expect("tool should convert validation failure into tool result");

        assert!(!result.ok);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("invalid spawnAgent params"))
        );
    }

    #[test]
    fn tool_description_is_stable_and_excludes_dynamic_profile_listing() {
        let executor = Arc::new(RecordingExecutor {
            calls: Mutex::new(Vec::new()),
        });
        let tool = SpawnAgentTool::new(executor);

        let definition = tool.definition();

        assert!(!definition.description.contains("## 可用的子 Agent"));
        assert!(!definition.description.contains("当前没有可用的子 Agent"));
        assert!(definition.description.contains("何时使用"));
        assert!(definition.description.contains("写清楚任务"));
        assert!(definition.description.contains("并行执行"));
        assert!(definition.description.contains("链式执行"));
    }

    #[tokio::test]
    async fn spawn_agent_tool_preserves_running_outcome_in_metadata() {
        struct RunningExecutor;

        #[async_trait]
        impl SubAgentExecutor for RunningExecutor {
            async fn launch(
                &self,
                _params: SpawnAgentParams,
                _ctx: &ToolContext,
            ) -> astrcode_core::Result<SubRunResult> {
                Ok(SubRunResult {
                    status: SubRunOutcome::Running,
                    handoff: Some(SubRunHandoff {
                        summary: "running".to_string(),
                        findings: vec!["status=running".to_string()],
                        artifacts: Vec::new(),
                    }),
                    failure: None,
                })
            }
        }

        let tool = SpawnAgentTool::new(Arc::new(RunningExecutor));
        let result = tool
            .execute(
                "call-running".to_string(),
                json!({
                    "description": "background task",
                    "prompt": "one"
                }),
                &tool_context(),
            )
            .await
            .expect("running outcome should still serialize");

        assert!(result.ok);
        assert_eq!(
            result
                .metadata
                .as_ref()
                .and_then(|value| value.get("outcome")),
            Some(&json!("running"))
        );
    }

    #[tokio::test]
    async fn spawn_agent_tool_surfaces_failure_display_and_technical_messages_separately() {
        struct FailingExecutor;

        #[async_trait]
        impl SubAgentExecutor for FailingExecutor {
            async fn launch(
                &self,
                _params: SpawnAgentParams,
                _ctx: &ToolContext,
            ) -> astrcode_core::Result<SubRunResult> {
                Ok(SubRunResult {
                    status: SubRunOutcome::Failed,
                    handoff: None,
                    failure: Some(SubRunFailure {
                        code: SubRunFailureCode::Transport,
                        display_message: "子 Agent 调用模型时网络连接中断，未完成任务。"
                            .to_string(),
                        technical_message: "HTTP request error: failed to read anthropic response \
                                            stream"
                            .to_string(),
                        retryable: true,
                    }),
                })
            }
        }

        let tool = SpawnAgentTool::new(Arc::new(FailingExecutor));
        let result = tool
            .execute(
                "call-failed".to_string(),
                json!({
                    "description": "background task",
                    "prompt": "one"
                }),
                &tool_context(),
            )
            .await
            .expect("failed outcome should still serialize");

        assert!(!result.ok);
        assert_eq!(
            result.output,
            "子 Agent 调用模型时网络连接中断，未完成任务。"
        );
        assert_eq!(
            result.error.as_deref(),
            Some("HTTP request error: failed to read anthropic response stream")
        );
    }

    #[tokio::test]
    async fn spawn_agent_tool_background_returns_subrun_artifact() {
        struct BackgroundExecutor;

        #[async_trait]
        impl SubAgentExecutor for BackgroundExecutor {
            async fn launch(
                &self,
                _params: SpawnAgentParams,
                _ctx: &ToolContext,
            ) -> astrcode_core::Result<SubRunResult> {
                Ok(SubRunResult {
                    status: SubRunOutcome::Running,
                    handoff: Some(SubRunHandoff {
                        summary: "spawnAgent 已在后台启动。".to_string(),
                        findings: Vec::new(),
                        artifacts: vec![astrcode_core::ArtifactRef {
                            kind: "subRun".to_string(),
                            id: "subrun-42".to_string(),
                            label: "Background sub-run".to_string(),
                            session_id: None,
                            storage_seq: None,
                            uri: None,
                        }],
                    }),
                    failure: None,
                })
            }
        }

        let tool = SpawnAgentTool::new(Arc::new(BackgroundExecutor));
        let result = tool
            .execute(
                "call-background".to_string(),
                json!({
                    "description": "background task",
                    "prompt": "one"
                }),
                &tool_context(),
            )
            .await
            .expect("background outcome should serialize");

        assert!(result.ok);
        assert_eq!(result.output, "spawnAgent 已在后台启动。");
        let artifact_kind = result
            .metadata
            .as_ref()
            .and_then(|value| value.get("handoff"))
            .and_then(|value| value.get("artifacts"))
            .and_then(|value| value.as_array())
            .and_then(|artifacts| artifacts.first())
            .and_then(|artifact| artifact.get("kind"))
            .and_then(|value| value.as_str());
        assert_eq!(artifact_kind, Some("subRun"));
    }
}
