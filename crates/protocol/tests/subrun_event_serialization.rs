use astrcode_protocol::http::{
    AgentContextDto, AgentEventEnvelope, AgentEventPayload, InvocationKindDto,
    ResolvedExecutionLimitsDto, ResolvedSubagentContextOverridesDto, SubRunStorageModeDto,
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
        },
        resolved_limits: ResolvedExecutionLimitsDto {
            max_steps: Some(30),
            token_budget: None,
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
        },
        resolved_limits: ResolvedExecutionLimitsDto {
            max_steps: Some(50),
            token_budget: Some(100_000),
            allowed_tools: vec!["readFile".to_string(), "writeFile".to_string()],
        },
    };

    let json = serde_json::to_value(&original).expect("serialize payload");
    let roundtripped: AgentEventPayload =
        serde_json::from_value(json).expect("deserialize payload");

    assert_eq!(original, roundtripped);
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
                "maxSteps": 50,
                "allowedTools": ["readFile"]
            }
        }
    });

    let result: Result<AgentEventPayload, _> = serde_json::from_value(payload);

    assert!(result.is_err(), "snake_case 字段应被拒绝");
}
