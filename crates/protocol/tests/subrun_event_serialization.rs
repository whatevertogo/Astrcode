use astrcode_protocol::http::{
    AgentContextDto, AgentEventEnvelope, AgentEventPayload, InvocationKindDto,
    ResolvedExecutionLimitsDto, ResolvedSubagentContextOverridesDto, SubRunDescriptorDto,
    SubRunFailureCodeDto, SubRunFailureDto, SubRunHandoffDto, SubRunOutcomeDto, SubRunResultDto,
    SubRunStorageModeDto,
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
fn sub_run_started_payload_roundtrip() {
    let original = AgentEventPayload::SubRunStarted {
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
        resolved_overrides: ResolvedSubagentContextOverridesDto {
            storage_mode: SubRunStorageModeDto::SharedSession,
            inherit_system_instructions: true,
            inherit_project_instructions: false,
            inherit_working_dir: true,
            inherit_policy_upper_bound: false,
            inherit_cancel_token: true,
            include_compact_summary: true,
            include_recent_tail: false,
            include_recovery_refs: true,
            include_parent_findings: false,
            fork_mode: None,
        },
        resolved_limits: ResolvedExecutionLimitsDto {
            allowed_tools: vec!["readFile".to_string(), "writeFile".to_string()],
        },
    };

    let json = serde_json::to_value(&original).expect("serialize payload");
    let roundtripped: AgentEventPayload =
        serde_json::from_value(json).expect("deserialize payload");

    assert_eq!(original, roundtripped);
}

#[test]
fn sub_run_started_serializes_descriptor_and_tool_call_id_in_camel_case() {
    let payload = AgentEventPayload::SubRunStarted {
        turn_id: Some("turn-1".to_string()),
        agent: AgentContextDto::default(),
        descriptor: Some(SubRunDescriptorDto {
            sub_run_id: "subrun-1".to_string(),
            parent_turn_id: "parent-turn".to_string(),
            parent_agent_id: None,
            depth: 1,
        }),
        tool_call_id: Some("call-1".to_string()),
        resolved_overrides: ResolvedSubagentContextOverridesDto {
            storage_mode: SubRunStorageModeDto::SharedSession,
            inherit_system_instructions: true,
            inherit_project_instructions: true,
            inherit_working_dir: true,
            inherit_policy_upper_bound: true,
            inherit_cancel_token: true,
            include_compact_summary: false,
            include_recent_tail: true,
            include_recovery_refs: false,
            include_parent_findings: false,
            fork_mode: None,
        },
        resolved_limits: ResolvedExecutionLimitsDto {
            allowed_tools: vec!["readFile".to_string()],
        },
    };

    let encoded = serde_json::to_value(AgentEventEnvelope::new(payload)).expect("serialize");
    let data = encoded.get("data").expect("data should exist");
    let descriptor = data.get("descriptor").expect("descriptor should exist");

    assert_eq!(
        descriptor.get("subRunId"),
        Some(&serde_json::json!("subrun-1"))
    );
    assert_eq!(
        descriptor.get("parentTurnId"),
        Some(&serde_json::json!("parent-turn"))
    );
    assert_eq!(descriptor.get("depth"), Some(&serde_json::json!(1)));
    assert_eq!(data.get("toolCallId"), Some(&serde_json::json!("call-1")));
    assert!(data.get("tool_call_id").is_none());
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
fn sub_run_started_omits_descriptor_field_when_none() {
    let payload = AgentEventPayload::SubRunStarted {
        turn_id: Some("turn-1".to_string()),
        agent: AgentContextDto::default(),
        descriptor: None,
        tool_call_id: None,
        resolved_overrides: ResolvedSubagentContextOverridesDto {
            storage_mode: SubRunStorageModeDto::SharedSession,
            inherit_system_instructions: true,
            inherit_project_instructions: true,
            inherit_working_dir: true,
            inherit_policy_upper_bound: true,
            inherit_cancel_token: true,
            include_compact_summary: false,
            include_recent_tail: false,
            include_recovery_refs: false,
            include_parent_findings: false,
            fork_mode: None,
        },
        resolved_limits: ResolvedExecutionLimitsDto {
            allowed_tools: vec![],
        },
    };

    let json = serde_json::to_value(AgentEventEnvelope::new(payload)).expect("serialize");
    let data = json.get("data").expect("data should exist");

    // Why: descriptor: None 应该省略字段，而非序列化为 null
    // 这样 frontend 可以用 `descriptor === undefined` 判断 legacy 事件
    assert!(
        !data.as_object().unwrap().contains_key("descriptor"),
        "descriptor field should be omitted when None, not serialized as null"
    );
    assert!(
        !data.as_object().unwrap().contains_key("toolCallId"),
        "toolCallId field should be omitted when None, not serialized as null"
    );
}

#[test]
fn sub_run_finished_omits_descriptor_field_when_none() {
    let payload = AgentEventPayload::SubRunFinished {
        turn_id: Some("turn-1".to_string()),
        agent: AgentContextDto::default(),
        descriptor: None,
        tool_call_id: None,
        result: SubRunResultDto {
            status: SubRunOutcomeDto::Completed,
            handoff: None,
            failure: None,
        },
        step_count: 5,
        estimated_tokens: 100,
    };

    let json = serde_json::to_value(AgentEventEnvelope::new(payload)).expect("serialize");
    let data = json.get("data").expect("data should exist");

    assert!(
        !data.as_object().unwrap().contains_key("descriptor"),
        "descriptor field should be omitted when None, not serialized as null"
    );
    assert!(
        !data.as_object().unwrap().contains_key("toolCallId"),
        "toolCallId field should be omitted when None, not serialized as null"
    );
}
