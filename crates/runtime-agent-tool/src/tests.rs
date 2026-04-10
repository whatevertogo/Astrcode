use std::sync::{Arc, Mutex};

use astrcode_core::{
    AgentStatus, ArtifactRef, CancelToken, ChildAgentRef, ChildSessionLineageKind,
    CloseAgentParams, CollaborationResult, CollaborationResultKind, DeliverToParentParams,
    ResumeAgentParams, SendAgentParams, SubRunFailure, SubRunFailureCode, SubRunHandoff,
    SubRunResult, Tool, ToolContext, WaitAgentParams, WaitUntil,
};
use async_trait::async_trait;
use serde_json::json;

use crate::{
    CloseAgentTool, CollaborationExecutor, DeliverToParentTool, ResumeAgentTool, SendAgentTool,
    SpawnAgentParams, SpawnAgentTool, SubAgentExecutor, WaitAgentTool,
};

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
            status: AgentStatus::Completed,
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
                status: AgentStatus::Running,
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
                status: AgentStatus::Failed,
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
                status: AgentStatus::Running,
                handoff: Some(SubRunHandoff {
                    summary: "spawnAgent 已在后台启动。".to_string(),
                    findings: Vec::new(),
                    artifacts: vec![
                        ArtifactRef {
                            kind: "subRun".to_string(),
                            id: "subrun-42".to_string(),
                            label: "Background sub-run".to_string(),
                            session_id: None,
                            storage_seq: None,
                            uri: None,
                        },
                        ArtifactRef {
                            kind: "agent".to_string(),
                            id: "agent-42".to_string(),
                            label: "Child agent id".to_string(),
                            session_id: None,
                            storage_seq: None,
                            uri: None,
                        },
                        ArtifactRef {
                            kind: "parentSession".to_string(),
                            id: "session-parent-42".to_string(),
                            label: "Parent session".to_string(),
                            session_id: Some("session-parent-42".to_string()),
                            storage_seq: None,
                            uri: None,
                        },
                        ArtifactRef {
                            kind: "session".to_string(),
                            id: "session-child-42".to_string(),
                            label: "Independent child session".to_string(),
                            session_id: Some("session-child-42".to_string()),
                            storage_seq: None,
                            uri: None,
                        },
                    ],
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
    assert_eq!(
        result
            .metadata
            .as_ref()
            .and_then(|value| value.get("openSessionId"))
            .and_then(|value| value.as_str()),
        Some("session-child-42")
    );
    assert_eq!(
        result
            .metadata
            .as_ref()
            .and_then(|value| value.get("agentRef"))
            .and_then(|value| value.get("agentId"))
            .and_then(|value| value.as_str()),
        Some("agent-42")
    );
}

// ─── 协作工具测试 ───────────────────────────────────────────

/// 记录所有调用并返回预设结果的协作执行器。
struct RecordingCollabExecutor {
    send_calls: Mutex<Vec<SendAgentParams>>,
    wait_calls: Mutex<Vec<WaitAgentParams>>,
    close_calls: Mutex<Vec<CloseAgentParams>>,
    resume_calls: Mutex<Vec<ResumeAgentParams>>,
    deliver_calls: Mutex<Vec<DeliverToParentParams>>,
}

impl RecordingCollabExecutor {
    fn new() -> Self {
        Self {
            send_calls: Mutex::new(Vec::new()),
            wait_calls: Mutex::new(Vec::new()),
            close_calls: Mutex::new(Vec::new()),
            resume_calls: Mutex::new(Vec::new()),
            deliver_calls: Mutex::new(Vec::new()),
        }
    }
}

fn sample_child_ref() -> ChildAgentRef {
    ChildAgentRef {
        agent_id: "agent-42".to_string(),
        session_id: "session-parent".to_string(),
        sub_run_id: "subrun-42".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        lineage_kind: ChildSessionLineageKind::Spawn,
        status: AgentStatus::Running,
        open_session_id: "session-child-42".to_string(),
    }
}

#[async_trait]
impl CollaborationExecutor for RecordingCollabExecutor {
    async fn send(
        &self,
        params: SendAgentParams,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<CollaborationResult> {
        self.send_calls.lock().expect("lock").push(params);
        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Sent,
            agent_ref: Some(sample_child_ref()),
            delivery_id: Some("delivery-1".to_string()),
            summary: Some("消息已发送".to_string()),
            parent_agent_id: None,
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    async fn wait(
        &self,
        params: WaitAgentParams,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<CollaborationResult> {
        self.wait_calls.lock().expect("lock").push(params);
        let mut child_ref = sample_child_ref();
        child_ref.status = AgentStatus::Completed;
        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::WaitResolved,
            agent_ref: Some(child_ref),
            delivery_id: None,
            summary: Some("子 Agent 已完成".to_string()),
            parent_agent_id: None,
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    async fn close(
        &self,
        params: CloseAgentParams,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<CollaborationResult> {
        self.close_calls.lock().expect("lock").push(params);
        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Closed,
            agent_ref: None,
            delivery_id: None,
            summary: Some("子 Agent 已关闭".to_string()),
            parent_agent_id: None,
            cascade: Some(true),
            closed_root_agent_id: Some("agent-42".to_string()),
            failure: None,
        })
    }

    async fn resume(
        &self,
        params: ResumeAgentParams,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<CollaborationResult> {
        self.resume_calls.lock().expect("lock").push(params);
        let mut child_ref = sample_child_ref();
        child_ref.status = AgentStatus::Running;
        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Resumed,
            agent_ref: Some(child_ref),
            delivery_id: None,
            summary: Some("子 Agent 已恢复".to_string()),
            parent_agent_id: None,
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }

    async fn deliver(
        &self,
        params: DeliverToParentParams,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<CollaborationResult> {
        self.deliver_calls.lock().expect("lock").push(params);
        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Delivered,
            agent_ref: None,
            delivery_id: Some("delivery-99".to_string()),
            summary: Some("结果已交付".to_string()),
            parent_agent_id: Some("agent-parent".to_string()),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }
}

// ─── sendAgent ──────────────────────────────────────────────

#[tokio::test]
async fn send_agent_tool_parses_params_and_delegates_to_executor() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = SendAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-send-1".to_string(),
            json!({
                "agentId": "agent-42",
                "message": "请修改第三部分",
                "context": "关注性能"
            }),
            &tool_context(),
        )
        .await
        .expect("sendAgent should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "消息已发送");
    assert_eq!(result.tool_name, "sendAgent");
    let calls = executor.send_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].agent_id, "agent-42");
    assert_eq!(calls[0].message, "请修改第三部分");
    assert_eq!(calls[0].context.as_deref(), Some("关注性能"));
}

#[tokio::test]
async fn send_agent_tool_rejects_missing_agent_id() {
    let tool = SendAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-send-2".to_string(),
            json!({"message": "hello"}),
            &tool_context(),
        )
        .await
        .expect("should return tool result");

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("invalid sendAgent params"))
    );
}

#[tokio::test]
async fn send_agent_tool_rejects_empty_message() {
    let tool = SendAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-send-3".to_string(),
            json!({"agentId": "agent-42", "message": "  "}),
            &tool_context(),
        )
        .await
        .expect("should return tool result");

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("invalid sendAgent params"))
    );
}

// ─── waitAgent ──────────────────────────────────────────────

#[tokio::test]
async fn wait_agent_tool_parses_params_and_returns_resolved_status() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = WaitAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-wait-1".to_string(),
            json!({"agentId": "agent-42"}),
            &tool_context(),
        )
        .await
        .expect("waitAgent should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "子 Agent 已完成");
    assert_eq!(result.tool_name, "waitAgent");
    let calls = executor.wait_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].agent_id, "agent-42");
    assert_eq!(calls[0].until, WaitUntil::Final);
}

#[tokio::test]
async fn wait_agent_tool_accepts_next_delivery_until() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = WaitAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-wait-2".to_string(),
            json!({"agentId": "agent-42", "until": "next_delivery"}),
            &tool_context(),
        )
        .await
        .expect("waitAgent should succeed");

    assert!(result.ok);
    let calls = executor.wait_calls.lock().expect("lock");
    assert_eq!(calls[0].until, WaitUntil::NextDelivery);
}

#[tokio::test]
async fn wait_agent_tool_rejects_missing_agent_id() {
    let tool = WaitAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute("call-wait-3".to_string(), json!({}), &tool_context())
        .await
        .expect("should return tool result");

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("invalid waitAgent params"))
    );
}

// ─── closeAgent ─────────────────────────────────────────────

#[tokio::test]
async fn close_agent_tool_parses_params_and_returns_cascade_info() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = CloseAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-close-1".to_string(),
            json!({"agentId": "agent-42"}),
            &tool_context(),
        )
        .await
        .expect("closeAgent should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "子 Agent 已关闭");
    assert_eq!(result.tool_name, "closeAgent");
    let calls = executor.close_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].cascade); // 默认级联
}

#[tokio::test]
async fn close_agent_tool_allows_disabling_cascade() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = CloseAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-close-2".to_string(),
            json!({"agentId": "agent-42", "cascade": false}),
            &tool_context(),
        )
        .await
        .expect("closeAgent should succeed");

    assert!(result.ok);
    let calls = executor.close_calls.lock().expect("lock");
    assert!(!calls[0].cascade);
}

#[tokio::test]
async fn close_agent_tool_rejects_empty_agent_id() {
    let tool = CloseAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-close-3".to_string(),
            json!({"agentId": "  "}),
            &tool_context(),
        )
        .await
        .expect("should return tool result");

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("invalid closeAgent params"))
    );
}

// ─── resumeAgent ────────────────────────────────────────────

#[tokio::test]
async fn resume_agent_tool_parses_params_and_returns_running_status() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = ResumeAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-resume-1".to_string(),
            json!({"agentId": "agent-42", "message": "请继续修改"}),
            &tool_context(),
        )
        .await
        .expect("resumeAgent should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "子 Agent 已恢复");
    assert_eq!(result.tool_name, "resumeAgent");
    let calls = executor.resume_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].agent_id, "agent-42");
    assert_eq!(calls[0].message.as_deref(), Some("请继续修改"));
}

#[tokio::test]
async fn resume_agent_tool_accepts_message_optional() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = ResumeAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-resume-2".to_string(),
            json!({"agentId": "agent-42"}),
            &tool_context(),
        )
        .await
        .expect("resumeAgent should succeed without message");

    assert!(result.ok);
    let calls = executor.resume_calls.lock().expect("lock");
    assert!(calls[0].message.is_none());
}

#[tokio::test]
async fn resume_agent_tool_rejects_empty_agent_id() {
    let tool = ResumeAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-resume-3".to_string(),
            json!({"agentId": ""}),
            &tool_context(),
        )
        .await
        .expect("should return tool result");

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("invalid resumeAgent params"))
    );
}

// ─── deliverToParent ────────────────────────────────────────

#[tokio::test]
async fn deliver_to_parent_tool_parses_params_and_returns_delivery_id() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = DeliverToParentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-deliver-1".to_string(),
            json!({
                "summary": "代码审查完成",
                "findings": ["发现3个问题"],
                "finalReply": "建议修改A模块"
            }),
            &tool_context(),
        )
        .await
        .expect("deliverToParent should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "结果已交付");
    assert_eq!(result.tool_name, "deliverToParent");
    let calls = executor.deliver_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].summary, "代码审查完成");
    assert_eq!(calls[0].findings, vec!["发现3个问题"]);
    assert_eq!(calls[0].final_reply.as_deref(), Some("建议修改A模块"));
}

#[tokio::test]
async fn deliver_to_parent_tool_rejects_empty_summary() {
    let tool = DeliverToParentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-deliver-2".to_string(),
            json!({"summary": "  "}),
            &tool_context(),
        )
        .await
        .expect("should return tool result");

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("invalid deliverToParent params"))
    );
}

#[tokio::test]
async fn deliver_to_parent_tool_works_with_summary_only() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = DeliverToParentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-deliver-3".to_string(),
            json!({"summary": "阶段性完成"}),
            &tool_context(),
        )
        .await
        .expect("deliverToParent should work with summary only");

    assert!(result.ok);
    let calls = executor.deliver_calls.lock().expect("lock");
    assert!(calls[0].findings.is_empty());
    assert!(calls[0].final_reply.is_none());
    assert!(calls[0].artifacts.is_empty());
}

// ─── 协作工具 metadata 不暴露 runtime 内部细节 ──────────────

#[test]
fn collaboration_tool_definitions_exclude_runtime_internals() {
    let executor = Arc::new(RecordingCollabExecutor::new());

    let send_def = SendAgentTool::new(executor.clone()).definition();
    assert!(!send_def.description.contains("AgentControl"));
    assert!(!send_def.description.contains("AgentInboxEnvelope"));

    let wait_def = WaitAgentTool::new(executor.clone()).definition();
    assert!(!wait_def.description.contains("runtime"));

    let close_def = CloseAgentTool::new(executor.clone()).definition();
    assert!(!close_def.description.contains("CancelToken"));

    let resume_def = ResumeAgentTool::new(executor.clone()).definition();
    assert!(!resume_def.description.contains("SubRunHandle"));

    let deliver_def = DeliverToParentTool::new(executor).definition();
    assert!(
        !deliver_def
            .description
            .contains("CollaborationNotification")
    );
}
