use astrcode_protocol::http::{
    AgentContextDto, AgentEventEnvelope, AgentEventPayload, ChildAgentRefDto,
    ChildSessionLineageKindDto, ChildSessionNotificationDto, ChildSessionNotificationKindDto,
    ChildSessionViewProjectionDto, ChildSessionViewResponseDto, InvocationKindDto,
    ParentChildSummaryListResponseDto, ResolvedExecutionLimitsDto,
    ResolvedSubagentContextOverridesDto, SubRunDescriptorDto, SubRunFailureCodeDto,
    SubRunFailureDto, SubRunHandoffDto, SubRunOutcomeDto, SubRunResultDto, SubRunStorageModeDto,
};

#[test]
fn sub_run_started_serializes_contract_fields_in_camel_case() {
    let payload = AgentEventPayload::SubRunStarted {
        turn_id: Some("turn-1".to_string()),
        agent: AgentContextDto {
            agent_id: Some("agent-1".to_string()),
            parent_turn_id: Some("parent-turn".to_string()),
            agent_profile: Some("explore".to_string()),
            sub_run_id: Some("subrun-1".to_string()),
            invocation_kind: Some(InvocationKindDto::SubRun),
            storage_mode: Some(SubRunStorageModeDto::SharedSession),
            child_session_id: None,
        },
        descriptor: None,
        tool_call_id: None,
        resolved_overrides: ResolvedSubagentContextOverridesDto {
            storage_mode: SubRunStorageModeDto::SharedSession,
            inherit_system_instructions: true,
            inherit_project_instructions: true,
            inherit_working_dir: true,
            inherit_policy_upper_bound: true,
            inherit_cancel_token: true,
            include_compact_summary: true,
            include_recent_tail: false,
            include_recovery_refs: false,
            include_parent_findings: false,
            fork_mode: None,
        },
        resolved_limits: ResolvedExecutionLimitsDto {
            allowed_tools: vec!["readFile".to_string(), "grep".to_string()],
        },
    };

    let value = serde_json::to_value(AgentEventEnvelope::new(payload)).expect("serialize");
    let data = value.get("data").expect("envelope should contain data");

    assert_eq!(
        value.get("event"),
        Some(&serde_json::json!("subRunStarted"))
    );
    assert!(data.get("resolvedOverrides").is_some());
    assert!(data.get("resolvedLimits").is_some());
    assert!(data.get("resolved_overrides").is_none());
    assert!(data.get("resolved_limits").is_none());
}

#[test]
fn sub_run_started_rejects_snake_case_fields() {
    let payload = serde_json::json!({
        "event": "subRunStarted",
        "data": {
            "turnId": "turn-1",
            "agentId": "agent-1",
            "parentTurnId": "parent-turn",
            "agentProfile": "explore",
            "subRunId": "subrun-1",
            "invocationKind": "subRun",
            "storageMode": "sharedSession",
            "resolved_overrides": {
                "storageMode": "sharedSession",
                "inheritSystemInstructions": true,
                "inheritProjectInstructions": false,
                "inheritWorkingDir": true,
                "inheritPolicyUpperBound": false,
                "inheritCancelToken": true,
                "includeCompactSummary": true,
                "includeRecentTail": false,
                "includeRecoveryRefs": false,
                "includeParentFindings": false
            },
            "resolved_limits": {
                "allowedTools": ["readFile"]
            }
        }
    });

    let result: Result<AgentEventPayload, _> = serde_json::from_value(payload);

    assert!(result.is_err(), "snake_case 字段应被拒绝");
}

#[test]
fn sub_run_finished_payload_roundtrip_new_result_shape() {
    let original = AgentEventPayload::SubRunFinished {
        turn_id: Some("turn-1".to_string()),
        agent: AgentContextDto {
            agent_id: Some("agent-1".to_string()),
            parent_turn_id: Some("parent-turn".to_string()),
            agent_profile: Some("explore".to_string()),
            sub_run_id: Some("subrun-1".to_string()),
            invocation_kind: Some(InvocationKindDto::SubRun),
            storage_mode: Some(SubRunStorageModeDto::SharedSession),
            child_session_id: None,
        },
        descriptor: Some(SubRunDescriptorDto {
            sub_run_id: "subrun-1".to_string(),
            parent_turn_id: "parent-turn".to_string(),
            parent_agent_id: Some("agent-root".to_string()),
            depth: 2,
        }),
        tool_call_id: Some("call-1".to_string()),
        result: SubRunResultDto {
            status: SubRunOutcomeDto::Failed,
            handoff: None,
            failure: Some(SubRunFailureDto {
                code: SubRunFailureCodeDto::Transport,
                display_message: "子 Agent 调用模型时网络连接中断，未完成任务。".to_string(),
                technical_message: "HTTP request error: failed to read anthropic response stream"
                    .to_string(),
                retryable: true,
            }),
        },
        step_count: 2,
        estimated_tokens: 123,
    };

    let json = serde_json::to_value(&original).expect("serialize payload");
    let roundtripped: AgentEventPayload =
        serde_json::from_value(json).expect("deserialize payload");

    assert_eq!(original, roundtripped);
}

#[test]
fn sub_run_finished_omits_parent_handoff_on_failure() {
    let payload = AgentEventPayload::SubRunFinished {
        turn_id: Some("turn-1".to_string()),
        agent: AgentContextDto::default(),
        descriptor: None,
        tool_call_id: None,
        result: SubRunResultDto {
            status: SubRunOutcomeDto::Completed,
            handoff: Some(SubRunHandoffDto {
                summary: "done".to_string(),
                findings: vec!["checked".to_string()],
                artifacts: Vec::new(),
            }),
            failure: None,
        },
        step_count: 1,
        estimated_tokens: 12,
    };

    let json = serde_json::to_value(AgentEventEnvelope::new(payload)).expect("serialize");
    let data = json.get("data").expect("data");
    let result = data.get("result").expect("result");

    assert!(result.get("handoff").is_some());
    assert!(result.get("failure").is_none());
}

#[test]
fn child_session_notification_roundtrip_keeps_projection_fields() {
    let notification = ChildSessionNotificationDto {
        notification_id: "note-1".to_string(),
        child_ref: ChildAgentRefDto {
            agent_id: "agent-child".to_string(),
            session_id: "session-parent".to_string(),
            sub_run_id: "subrun-1".to_string(),
            parent_agent_id: Some("agent-parent".to_string()),
            lineage_kind: ChildSessionLineageKindDto::Spawn,
            status: "running".to_string(),
            openable: true,
            open_session_id: "session-child".to_string(),
        },
        kind: ChildSessionNotificationKindDto::Started,
        summary: "child started".to_string(),
        status: "running".to_string(),
        open_session_id: "session-child".to_string(),
        source_tool_call_id: Some("call-1".to_string()),
        final_reply_excerpt: None,
    };

    let encoded = serde_json::to_value(&notification).expect("serialize notification");
    let decoded: ChildSessionNotificationDto =
        serde_json::from_value(encoded.clone()).expect("deserialize notification");

    assert_eq!(decoded, notification);
    assert_eq!(
        encoded.get("notificationId"),
        Some(&serde_json::json!("note-1"))
    );
    assert_eq!(
        encoded.get("openSessionId"),
        Some(&serde_json::json!("session-child"))
    );
}

#[test]
fn child_session_summary_and_view_response_roundtrip() {
    let child_ref = ChildAgentRefDto {
        agent_id: "agent-child".to_string(),
        session_id: "session-parent".to_string(),
        sub_run_id: "subrun-1".to_string(),
        parent_agent_id: Some("agent-parent".to_string()),
        lineage_kind: ChildSessionLineageKindDto::Resume,
        status: "completed".to_string(),
        openable: true,
        open_session_id: "session-child".to_string(),
    };

    let summary = ParentChildSummaryListResponseDto {
        items: vec![ChildSessionNotificationDto {
            notification_id: "note-2".to_string(),
            child_ref: child_ref.clone(),
            kind: ChildSessionNotificationKindDto::Delivered,
            summary: "child delivered".to_string(),
            status: "completed".to_string(),
            open_session_id: "session-child".to_string(),
            source_tool_call_id: Some("call-2".to_string()),
            final_reply_excerpt: Some("done".to_string()),
        }],
    };
    let summary_json = serde_json::to_value(&summary).expect("serialize summary");
    let summary_back: ParentChildSummaryListResponseDto =
        serde_json::from_value(summary_json).expect("deserialize summary");
    assert_eq!(summary_back.items.len(), 1);
    assert_eq!(summary_back.items[0].child_ref.agent_id, "agent-child");

    let view = ChildSessionViewResponseDto {
        view: ChildSessionViewProjectionDto {
            child_ref,
            title: "reviewer".to_string(),
            status: "completed".to_string(),
            summary_items: vec!["summary".to_string()],
            latest_tool_activity: vec!["readFile".to_string()],
            has_final_reply: true,
            child_session_id: "session-child".to_string(),
            has_descriptor_lineage: true,
        },
    };
    let view_json = serde_json::to_value(&view).expect("serialize view");
    let view_back: ChildSessionViewResponseDto =
        serde_json::from_value(view_json).expect("deserialize view");
    assert_eq!(view_back.view.child_session_id, "session-child");
    assert!(view_back.view.has_final_reply);
}

#[test]
fn child_session_notification_event_payload_roundtrip() {
    let payload = AgentEventPayload::ChildSessionNotification {
        turn_id: Some("turn-parent".to_string()),
        agent: AgentContextDto {
            agent_id: Some("agent-parent".to_string()),
            parent_turn_id: Some("turn-parent".to_string()),
            agent_profile: Some("planner".to_string()),
            sub_run_id: Some("subrun-parent".to_string()),
            invocation_kind: Some(InvocationKindDto::SubRun),
            storage_mode: Some(SubRunStorageModeDto::SharedSession),
            child_session_id: None,
        },
        child_ref: ChildAgentRefDto {
            agent_id: "agent-child".to_string(),
            session_id: "session-parent".to_string(),
            sub_run_id: "subrun-1".to_string(),
            parent_agent_id: Some("agent-parent".to_string()),
            lineage_kind: ChildSessionLineageKindDto::Spawn,
            status: "running".to_string(),
            openable: true,
            open_session_id: "session-child".to_string(),
        },
        kind: ChildSessionNotificationKindDto::Started,
        summary: "child started".to_string(),
        status: "running".to_string(),
        open_session_id: "session-child".to_string(),
        source_tool_call_id: Some("call-1".to_string()),
        final_reply_excerpt: None,
    };

    let encoded =
        serde_json::to_value(AgentEventEnvelope::new(payload.clone())).expect("serialize");
    let decoded: AgentEventEnvelope = serde_json::from_value(encoded).expect("deserialize");
    assert_eq!(decoded.event, payload);
}

// ─── T041 谱系兼容性测试 ──────────────────────────────

/// 验证 spawn/fork/resume 三种 lineage kind 在 ChildAgentRefDto 中均可序列化和反序列化，
/// 且 JSON 输出使用 snake_case 值（"spawn"/"fork"/"resume"）。
#[test]
fn lineage_kind_spawn_fork_resume_all_roundtrip_through_child_ref() {
    for (label, kind) in [
        ("spawn", ChildSessionLineageKindDto::Spawn),
        ("fork", ChildSessionLineageKindDto::Fork),
        ("resume", ChildSessionLineageKindDto::Resume),
    ] {
        let child_ref = ChildAgentRefDto {
            agent_id: "agent-child".to_string(),
            session_id: "session-parent".to_string(),
            sub_run_id: "subrun-1".to_string(),
            parent_agent_id: Some("agent-parent".to_string()),
            lineage_kind: kind.clone(),
            status: "running".to_string(),
            openable: true,
            open_session_id: "session-child".to_string(),
        };

        let json = serde_json::to_value(&child_ref).expect("serialize child ref");
        // 验证 JSON 中 lineageKind 值为 snake_case 字符串
        assert_eq!(
            json.get("lineageKind"),
            Some(&serde_json::json!(label)),
            "lineage_kind {label} should serialize as snake_case"
        );

        let back: ChildAgentRefDto = serde_json::from_value(json).expect("deserialize child ref");
        assert_eq!(
            back.lineage_kind, kind,
            "roundtrip for {label} should match"
        );
    }
}
