use std::sync::{Arc, Mutex};

use astrcode_core::{
    ArtifactRef, CancelToken, SubRunFailure, SubRunFailureCode, SubRunHandoff, SubRunOutcome,
    SubRunResult, Tool, ToolContext,
};
use async_trait::async_trait;
use serde_json::json;

use crate::{SpawnAgentParams, SpawnAgentTool, SubAgentExecutor};

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
                    display_message: "子 Agent 调用模型时网络连接中断，未完成任务。".to_string(),
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
                    artifacts: vec![ArtifactRef {
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
