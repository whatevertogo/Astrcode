use std::sync::{Arc, Mutex};

use astrcode_core::{
    AgentLifecycleStatus, AgentTurnOutcome, ArtifactRef, CancelToken, ChildAgentRef,
    ChildSessionLineageKind, CloseAgentParams, CollaborationResult, CollaborationResultKind,
    CompletedParentDeliveryPayload, DelegationMetadata, ObserveParams, ParentDelivery,
    ParentDeliveryOrigin, ParentDeliveryPayload, ParentDeliveryTerminalSemantics,
    ProgressParentDeliveryPayload, SendAgentParams, SendToChildParams, SendToParentParams,
    SpawnAgentParams, SpawnCapabilityGrant, SubRunFailure, SubRunFailureCode, SubRunHandoff,
    SubRunResult, Tool, ToolContext,
};
use async_trait::async_trait;
use serde_json::json;

use crate::agent_tools::{
    CloseAgentTool, CollaborationExecutor, ObserveAgentTool, SendAgentTool, SpawnAgentTool,
    SubAgentExecutor, collab_result_mapping::map_collaboration_result,
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
            lifecycle: AgentLifecycleStatus::Idle,
            last_turn_outcome: Some(AgentTurnOutcome::Completed),
            handoff: Some(SubRunHandoff {
                findings: vec!["checked".to_string()],
                artifacts: Vec::new(),
                delivery: Some(ParentDelivery {
                    idempotency_key: "handoff-done".to_string(),
                    origin: ParentDeliveryOrigin::Explicit,
                    terminal_semantics: ParentDeliveryTerminalSemantics::Terminal,
                    source_turn_id: Some("turn-done".to_string()),
                    payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                        message: "done".to_string(),
                        findings: vec!["checked".to_string()],
                        artifacts: Vec::new(),
                    }),
                }),
            }),
            failure: None,
        })
    }
}

fn tool_context() -> ToolContext {
    ToolContext::new(
        "session-1".to_string().into(),
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
                "context": "focus on tests",
                "capabilityGrant": {
                    "allowedTools": ["grep", "readFile"]
                }
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
        calls[0].capability_grant,
        Some(SpawnCapabilityGrant {
            allowed_tools: vec!["grep".to_string(), "readFile".to_string()],
        })
    );
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
            .is_some_and(|error| error.contains("invalid spawn params"))
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
    assert!(
        definition
            .description
            .contains("one new isolated responsibility")
    );
    assert!(definition.description.contains("Start with one child"));
    assert!(definition.description.contains("Do not use `spawn`"));
}

#[test]
fn spawn_tool_exposes_prompt_metadata_for_tool_summary_indexing() {
    let executor = Arc::new(RecordingExecutor {
        calls: Mutex::new(Vec::new()),
    });
    let tool = SpawnAgentTool::new(executor);

    let prompt = tool
        .capability_metadata()
        .prompt
        .expect("spawn should expose prompt metadata");

    assert!(prompt.summary.contains("isolated context"));
    assert!(prompt.guide.contains("Start with one child"));
    assert!(prompt.guide.contains("`agentId`"));
    assert!(prompt.guide.contains("`capabilityGrant`"));
}

#[test]
fn send_observe_close_prompt_metadata_stays_action_oriented() {
    let executor = Arc::new(RecordingCollabExecutor::new());

    let send_prompt = SendAgentTool::new(executor.clone())
        .capability_metadata()
        .prompt
        .expect("send should expose prompt metadata");
    assert!(send_prompt.summary.contains("upstream typed delivery"));
    assert!(send_prompt.guide.contains("direct child"));
    assert!(send_prompt.guide.contains("direct parent"));
    assert!(send_prompt.guide.contains("both directions in one turn"));

    let observe_prompt = ObserveAgentTool::new(executor.clone())
        .capability_metadata()
        .prompt
        .expect("observe should expose prompt metadata");
    assert!(observe_prompt.summary.contains("decide the next action"));
    assert!(observe_prompt.guide.contains("`wait`, `send`, or `close`"));
    assert!(observe_prompt.guide.contains("current child state"));
    assert!(
        !observe_prompt
            .guide
            .contains("Should I `send` another instruction")
    );

    let close_prompt = CloseAgentTool::new(executor)
        .capability_metadata()
        .prompt
        .expect("close should expose prompt metadata");
    assert!(
        close_prompt
            .summary
            .contains("finished or no longer useful")
    );
    assert!(close_prompt.guide.contains("cascade"));
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
                lifecycle: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
                handoff: Some(SubRunHandoff {
                    findings: vec!["status=running".to_string()],
                    artifacts: Vec::new(),
                    delivery: Some(ParentDelivery {
                        idempotency_key: "handoff-running".to_string(),
                        origin: ParentDeliveryOrigin::Explicit,
                        terminal_semantics: ParentDeliveryTerminalSemantics::NonTerminal,
                        source_turn_id: None,
                        payload: ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                            message: "running".to_string(),
                        }),
                    }),
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
                lifecycle: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Failed),
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
                lifecycle: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
                handoff: Some(SubRunHandoff {
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
                    delivery: Some(ParentDelivery {
                        idempotency_key: "handoff-subrun-42".to_string(),
                        origin: ParentDeliveryOrigin::Explicit,
                        terminal_semantics: ParentDeliveryTerminalSemantics::NonTerminal,
                        source_turn_id: None,
                        payload: ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                            message: "spawn 已在后台启动。".to_string(),
                        }),
                    }),
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
    assert_eq!(result.output, "spawn 已在后台启动。");
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

#[tokio::test]
async fn tool_flow_reuses_spawned_agent_id_for_send_and_close() {
    struct BackgroundExecutor;

    #[async_trait]
    impl SubAgentExecutor for BackgroundExecutor {
        async fn launch(
            &self,
            _params: SpawnAgentParams,
            _ctx: &ToolContext,
        ) -> astrcode_core::Result<SubRunResult> {
            Ok(SubRunResult {
                lifecycle: AgentLifecycleStatus::Running,
                last_turn_outcome: None,
                handoff: Some(SubRunHandoff {
                    findings: Vec::new(),
                    artifacts: vec![
                        ArtifactRef {
                            kind: "subRun".to_string(),
                            id: "subrun-99".to_string(),
                            label: "Background sub-run".to_string(),
                            session_id: None,
                            storage_seq: None,
                            uri: None,
                        },
                        ArtifactRef {
                            kind: "agent".to_string(),
                            id: "agent-99".to_string(),
                            label: "Child agent id".to_string(),
                            session_id: None,
                            storage_seq: None,
                            uri: None,
                        },
                        ArtifactRef {
                            kind: "parentSession".to_string(),
                            id: "session-parent-99".to_string(),
                            label: "Parent session".to_string(),
                            session_id: Some("session-parent-99".to_string()),
                            storage_seq: None,
                            uri: None,
                        },
                        ArtifactRef {
                            kind: "session".to_string(),
                            id: "session-child-99".to_string(),
                            label: "Independent child session".to_string(),
                            session_id: Some("session-child-99".to_string()),
                            storage_seq: None,
                            uri: None,
                        },
                    ],
                    delivery: Some(ParentDelivery {
                        idempotency_key: "handoff-subrun-99".to_string(),
                        origin: ParentDeliveryOrigin::Explicit,
                        terminal_semantics: ParentDeliveryTerminalSemantics::NonTerminal,
                        source_turn_id: None,
                        payload: ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                            message: "spawn 已在后台启动。".to_string(),
                        }),
                    }),
                }),
                failure: None,
            })
        }
    }

    let spawn_tool = SpawnAgentTool::new(Arc::new(BackgroundExecutor));
    let executor = Arc::new(RecordingCollabExecutor::new());
    let send_tool = SendAgentTool::new(executor.clone());
    let close_tool = CloseAgentTool::new(executor.clone());

    let spawned = spawn_tool
        .execute(
            "call-flow-spawn".to_string(),
            json!({
                "description": "background task",
                "prompt": "one"
            }),
            &tool_context(),
        )
        .await
        .expect("spawn should succeed");
    let spawned_agent_id = spawned
        .metadata
        .as_ref()
        .and_then(|value| value.get("agentRef"))
        .and_then(|value| value.get("agentId"))
        .and_then(|value| value.as_str())
        .expect("spawn should expose a stable agentId")
        .to_string();

    let send_result = send_tool
        .execute(
            "call-flow-send".to_string(),
            json!({
                "agentId": spawned_agent_id,
                "message": "继续执行第二轮"
            }),
            &tool_context(),
        )
        .await
        .expect("send should succeed");
    assert!(send_result.ok);

    let close_result = close_tool
        .execute(
            "call-flow-close".to_string(),
            json!({
                "agentId": "agent-99"
            }),
            &tool_context(),
        )
        .await
        .expect("close should succeed");
    assert!(close_result.ok);

    let send_calls = executor.send_calls.lock().expect("lock");
    assert_eq!(send_calls.len(), 1);
    assert!(matches!(
        &send_calls[0],
        SendAgentParams::ToChild(SendToChildParams { agent_id, .. }) if agent_id == "agent-99"
    ));
    drop(send_calls);

    let close_calls = executor.close_calls.lock().expect("lock");
    assert_eq!(close_calls.len(), 1);
    assert_eq!(close_calls[0].agent_id, "agent-99");
}

// ─── 协作工具测试 ───────────────────────────────────────────

/// 记录所有调用并返回预设结果的协作执行器。
struct RecordingCollabExecutor {
    send_calls: Mutex<Vec<SendAgentParams>>,
    close_calls: Mutex<Vec<CloseAgentParams>>,
    observe_calls: Mutex<Vec<ObserveParams>>,
}

impl RecordingCollabExecutor {
    fn new() -> Self {
        Self {
            send_calls: Mutex::new(Vec::new()),
            close_calls: Mutex::new(Vec::new()),
            observe_calls: Mutex::new(Vec::new()),
        }
    }
}

fn sample_child_ref() -> ChildAgentRef {
    ChildAgentRef {
        agent_id: "agent-42".to_string(),
        session_id: "session-parent".to_string(),
        sub_run_id: "subrun-42".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        parent_sub_run_id: Some("subrun-parent".to_string()),
        lineage_kind: ChildSessionLineageKind::Spawn,
        status: AgentLifecycleStatus::Running,
        open_session_id: "session-child-42".to_string(),
    }
}

fn sample_delegation(restricted: bool) -> DelegationMetadata {
    DelegationMetadata {
        responsibility_summary: "检查缓存层".to_string(),
        reuse_scope_summary: if restricted {
            "只有当下一步仍属于同一责任分支，且所需操作仍落在当前收缩后的 capability surface \
             内时，才应继续复用这个 child。"
                .to_string()
        } else {
            "只有当下一步仍属于同一责任分支时，才应继续复用这个 child；若责任边界已经改变，应 \
             close 当前分支并重新选择更合适的执行主体。"
                .to_string()
        },
        restricted,
        capability_limit_summary: restricted
            .then(|| "本分支当前只允许使用这些工具：readFile, grep。".to_string()),
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
            observe_result: None,
            delegation: Some(sample_delegation(false)),
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
            observe_result: None,
            delegation: None,
            cascade: Some(true),
            closed_root_agent_id: Some("agent-42".to_string()),
            failure: None,
        })
    }

    async fn observe(
        &self,
        params: ObserveParams,
        _ctx: &ToolContext,
    ) -> astrcode_core::Result<CollaborationResult> {
        let agent_id = params.agent_id.clone();
        self.observe_calls.lock().expect("lock").push(params);
        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Observed,
            agent_ref: Some(sample_child_ref()),
            delivery_id: None,
            summary: Some(format!(
                "子 Agent {} 当前为 Idle；建议 send_or_close：上一轮已完成。",
                agent_id
            )),
            observe_result: Some(astrcode_core::ObserveAgentResult {
                agent_id,
                sub_run_id: "subrun-42".to_string(),
                session_id: "session-parent".to_string(),
                open_session_id: "session-child-42".to_string(),
                parent_agent_id: "agent-parent".to_string(),
                lifecycle_status: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                phase: "Idle".to_string(),
                turn_count: 1,
                pending_message_count: 0,
                active_task: None,
                pending_task: None,
                recent_mailbox_messages: vec!["最近一条 mailbox 摘要".to_string()],
                last_output: Some("done".to_string()),
                delegation: Some(sample_delegation(false)),
                recommended_next_action: "send_or_close".to_string(),
                recommended_reason: "上一轮已完成。".to_string(),
                delivery_freshness: "ready_for_follow_up".to_string(),
            }),
            delegation: Some(sample_delegation(false)),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }
}

// ─── send ──────────────────────────────────────────────────

#[tokio::test]
async fn send_agent_tool_parses_downstream_params_and_delegates_to_executor() {
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
        .expect("send should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "消息已发送");
    assert_eq!(result.tool_name, "send");
    let calls = executor.send_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert!(matches!(
        &calls[0],
        SendAgentParams::ToChild(SendToChildParams {
            agent_id,
            message,
            context,
        }) if agent_id == "agent-42"
            && message == "请修改第三部分"
            && context.as_deref() == Some("关注性能")
    ));
}

#[tokio::test]
async fn send_agent_tool_parses_upstream_params_and_delegates_to_executor() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = SendAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-send-upstream".to_string(),
            json!({
                "kind": "completed",
                "payload": {
                    "message": "子任务已完成",
                    "findings": ["结论一"]
                }
            }),
            &tool_context(),
        )
        .await
        .expect("upstream send should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "消息已发送");
    let calls = executor.send_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert!(matches!(
        &calls[0],
        SendAgentParams::ToParent(SendToParentParams {
            payload: ParentDeliveryPayload::Completed(CompletedParentDeliveryPayload {
                message,
                findings,
                artifacts,
            })
        }) if message == "子任务已完成"
            && findings == &vec!["结论一".to_string()]
            && artifacts.is_empty()
    ));
}

#[tokio::test]
async fn send_agent_tool_rejects_missing_branch_shape() {
    let tool = SendAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-send-invalid".to_string(),
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
            .is_some_and(|e| e.contains("invalid send params"))
    );
}

#[tokio::test]
async fn send_agent_tool_rejects_empty_downstream_message() {
    let tool = SendAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-send-empty-downstream".to_string(),
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
            .is_some_and(|e| e.contains("invalid send params"))
    );
}

#[tokio::test]
async fn send_agent_tool_rejects_empty_upstream_message() {
    let tool = SendAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-send-empty-upstream".to_string(),
            json!({
                "kind": "progress",
                "payload": { "message": "  " }
            }),
            &tool_context(),
        )
        .await
        .expect("should return tool result");

    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|e| e.contains("invalid send params"))
    );
}

// ─── close ─────────────────────────────────────────────────

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
        .expect("close should succeed");

    assert!(result.ok);
    assert_eq!(result.output, "子 Agent 已关闭");
    assert_eq!(result.tool_name, "close");
    let calls = executor.close_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].agent_id, "agent-42");
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
            .is_some_and(|e| e.contains("invalid close params"))
    );
}

// ─── observe ───────────────────────────────────────────────

#[tokio::test]
async fn observe_agent_tool_parses_params_and_delegates_to_executor() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tool = ObserveAgentTool::new(executor.clone());

    let result = tool
        .execute(
            "call-observe-1".to_string(),
            json!({"agentId": "agent-42"}),
            &tool_context(),
        )
        .await
        .expect("observe should succeed");

    assert!(result.ok);
    assert_eq!(result.tool_name, "observe");
    assert!(
        result
            .metadata
            .as_ref()
            .and_then(|value| value.get("observeResult"))
            .and_then(|value| value.get("recommendedNextAction"))
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == "send_or_close")
    );
    let calls = executor.observe_calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].agent_id, "agent-42");
}

#[test]
fn collaboration_result_metadata_projects_idle_reuse_and_branch_mismatch_hints() {
    let mapped = map_collaboration_result(
        "call-observe-advisory".to_string(),
        "observe",
        CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Observed,
            agent_ref: Some(sample_child_ref()),
            delivery_id: None,
            summary: Some("子 Agent agent-42 当前为 Idle。".to_string()),
            observe_result: Some(astrcode_core::ObserveAgentResult {
                agent_id: "agent-42".to_string(),
                sub_run_id: "subrun-42".to_string(),
                session_id: "session-parent".to_string(),
                open_session_id: "session-child-42".to_string(),
                parent_agent_id: "agent-parent".to_string(),
                lifecycle_status: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                phase: "Idle".to_string(),
                turn_count: 1,
                pending_message_count: 0,
                active_task: None,
                pending_task: None,
                recent_mailbox_messages: Vec::new(),
                last_output: Some("done".to_string()),
                delegation: Some(sample_delegation(false)),
                recommended_next_action: "send_or_close".to_string(),
                recommended_reason: "同一责任分支可继续 send。".to_string(),
                delivery_freshness: "ready_for_follow_up".to_string(),
            }),
            delegation: Some(sample_delegation(false)),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        },
    );

    assert_eq!(
        mapped
            .metadata
            .as_ref()
            .and_then(|value| value.get("advisory"))
            .and_then(|value| value.get("branch"))
            .and_then(|value| value.get("sameResponsibilityAction"))
            .and_then(|value| value.as_str()),
        Some("send")
    );
    assert_eq!(
        mapped
            .metadata
            .as_ref()
            .and_then(|value| value.get("advisory"))
            .and_then(|value| value.get("branch"))
            .and_then(|value| value.get("differentResponsibilityAction"))
            .and_then(|value| value.as_str()),
        Some("close_or_respawn")
    );
}

#[test]
fn collaboration_result_metadata_projects_restricted_child_broader_tool_hint() {
    let mapped = map_collaboration_result(
        "call-observe-restricted".to_string(),
        "observe",
        CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Observed,
            agent_ref: Some(sample_child_ref()),
            delivery_id: None,
            summary: Some("restricted child idle".to_string()),
            observe_result: Some(astrcode_core::ObserveAgentResult {
                agent_id: "agent-42".to_string(),
                sub_run_id: "subrun-42".to_string(),
                session_id: "session-parent".to_string(),
                open_session_id: "session-child-42".to_string(),
                parent_agent_id: "agent-parent".to_string(),
                lifecycle_status: AgentLifecycleStatus::Idle,
                last_turn_outcome: Some(AgentTurnOutcome::Completed),
                phase: "Idle".to_string(),
                turn_count: 1,
                pending_message_count: 0,
                active_task: None,
                pending_task: None,
                recent_mailbox_messages: Vec::new(),
                last_output: Some("done".to_string()),
                delegation: Some(sample_delegation(true)),
                recommended_next_action: "send_or_close".to_string(),
                recommended_reason: "同一责任分支且工具面匹配时可继续复用。".to_string(),
                delivery_freshness: "ready_for_follow_up".to_string(),
            }),
            delegation: Some(sample_delegation(true)),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        },
    );

    assert_eq!(
        mapped
            .metadata
            .as_ref()
            .and_then(|value| value.get("advisory"))
            .and_then(|value| value.get("branch"))
            .and_then(|value| value.get("broaderToolsAction"))
            .and_then(|value| value.as_str()),
        Some("respawn_or_handle_here")
    );
    assert_eq!(
        mapped
            .metadata
            .as_ref()
            .and_then(|value| value.get("advisory"))
            .and_then(|value| value.get("branch"))
            .and_then(|value| value.get("capabilityLimitSummary"))
            .and_then(|value| value.as_str()),
        Some("本分支当前只允许使用这些工具：readFile, grep。")
    );
}

#[tokio::test]
async fn observe_agent_tool_rejects_empty_agent_id() {
    let tool = ObserveAgentTool::new(Arc::new(RecordingCollabExecutor::new()));

    let result = tool
        .execute(
            "call-observe-2".to_string(),
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
            .is_some_and(|e| e.contains("invalid observe params"))
    );
}

// ─── 协作工具公开面回归 ─────────────────────────────────────

#[test]
fn collaboration_prompt_metadata_stays_action_oriented() {
    let executor = Arc::new(RecordingCollabExecutor::new());

    let send_prompt = SendAgentTool::new(executor.clone())
        .capability_metadata()
        .prompt
        .expect("send should expose prompt metadata");
    assert!(send_prompt.summary.contains("upstream typed delivery"));
    assert!(send_prompt.guide.contains("direct child"));
    assert!(send_prompt.guide.contains("direct parent"));
    assert!(send_prompt.guide.contains("both directions in one turn"));

    let observe_prompt = ObserveAgentTool::new(executor.clone())
        .capability_metadata()
        .prompt
        .expect("observe should expose prompt metadata");
    assert!(observe_prompt.summary.contains("decide the next action"));
    assert!(observe_prompt.guide.contains("`wait`, `send`, or `close`"));
    assert!(observe_prompt.guide.contains("current child state"));

    let close_prompt = CloseAgentTool::new(executor)
        .capability_metadata()
        .prompt
        .expect("close should expose prompt metadata");
    assert!(
        close_prompt
            .summary
            .contains("finished or no longer useful")
    );
    assert!(close_prompt.guide.contains("cascade"));
}

#[test]
fn collaboration_tools_registered_in_public_surface() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(SendAgentTool::new(executor.clone())),
        Box::new(ObserveAgentTool::new(executor.clone())),
        Box::new(CloseAgentTool::new(executor)),
    ];

    let names: Vec<String> = tools.iter().map(|t| t.definition().name.clone()).collect();
    assert_eq!(names, vec!["send", "observe", "close"]);
}

#[test]
fn collaboration_tool_definitions_exclude_runtime_internals() {
    let executor = Arc::new(RecordingCollabExecutor::new());

    let send_def = SendAgentTool::new(executor.clone()).definition();
    assert!(!send_def.description.contains("AgentControl"));
    assert!(!send_def.description.contains("AgentInboxEnvelope"));

    let close_def = CloseAgentTool::new(executor.clone()).definition();
    assert!(!close_def.description.contains("CancelToken"));

    let observe_def = ObserveAgentTool::new(executor).definition();
    assert!(!observe_def.description.contains("MailboxProjection"));
}

#[test]
fn old_tool_names_not_in_definitions() {
    let executor = Arc::new(RecordingCollabExecutor::new());
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(SendAgentTool::new(executor.clone())),
        Box::new(ObserveAgentTool::new(executor.clone())),
        Box::new(CloseAgentTool::new(executor)),
    ];

    for tool in &tools {
        let name = &tool.definition().name;
        assert!(
            ![
                "waitAgent",
                "resumeAgent",
                "deliverToParent",
                "reply_to_parent",
                "spawnAgent",
                "sendAgent",
                "closeAgent"
            ]
            .contains(&name.as_str()),
            "old tool name '{}' should not appear",
            name
        );
    }
}
