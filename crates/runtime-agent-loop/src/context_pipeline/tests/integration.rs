use super::*;

fn compact_clearable_descriptor(name: &str) -> CapabilityDescriptor {
    CapabilityDescriptor::builder(name, CapabilityKind::tool())
        .description("test tool")
        .schema(json!({"type": "object"}), json!({"type": "string"}))
        .compact_clearable(true)
        .build()
        .expect("descriptor should build")
}

/// 辅助：构造包含多个工具调用的消息流。
/// 格式：[(tool_name, call_id, result_content), ...]
fn make_tool_conversation(calls: &[(&str, &str, &str)]) -> Vec<LlmMessage> {
    let mut messages = Vec::new();
    messages.push(LlmMessage::Assistant {
        content: String::new(),
        tool_calls: calls
            .iter()
            .map(|(name, id, _)| astrcode_core::ToolCallRequest {
                id: id.to_string(),
                name: name.to_string(),
                args: json!({}),
            })
            .collect(),
        reasoning: None,
    });
    for (_, id, content) in calls {
        messages.push(LlmMessage::Tool {
            tool_call_id: id.to_string(),
            content: content.to_string(),
        });
    }
    messages
}

#[test]
fn persistence_budget_persists_with_session_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let big = "x".repeat(40_000);
    let messages = make_tool_conversation(&[
        ("readFile", "call-1", &big),
        ("readFile", "call-2", "small result"),
    ]);

    let state = AgentState {
        session_id: "s-persist-test".to_string(),
        working_dir: working.clone(),
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: None,
    };

    let descriptors = vec![compact_clearable_descriptor("readFile")];

    let bundle = ContextRuntime::new(100_000)
        .with_persistence_budget_config(PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 10_000,
        })
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert!(bundle.persistence_stats.persisted_count >= 1);
    assert!(bundle.persistence_stats.bytes_saved > 0);
}

#[test]
fn persistence_budget_no_op_without_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let big = "x".repeat(40_000);
    let messages = make_tool_conversation(&[("readFile", "call-1", &big)]);

    let state = AgentState {
        session_id: "s-no-config".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: None,
    };

    let bundle = ContextRuntime::new(100_000)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &[],
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.persistence_stats.persisted_count, 0);
}

#[test]
fn micro_compact_clears_when_gap_exceeded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let messages = make_tool_conversation(&[
        ("readFile", "call-old", "old file content"),
        ("readFile", "call-recent", "recent file content"),
    ]);

    let past = chrono::Utc::now() - chrono::Duration::seconds(7200);

    let state = AgentState {
        session_id: "s-micro-test".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: Some(past),
    };

    let descriptors = vec![compact_clearable_descriptor("readFile")];

    let bundle = ContextRuntime::new(100_000)
        .with_micro_compact_config(MicroCompactConfig {
            gap_threshold_secs: 3600,
            keep_recent_results: 1,
        })
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert!(bundle.micro_compact_stats.cleared_count >= 1);
}

#[test]
fn micro_compact_no_op_when_gap_below_threshold() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let messages = make_tool_conversation(&[("readFile", "call-1", "content")]);
    let recent = chrono::Utc::now() - chrono::Duration::seconds(10);

    let state = AgentState {
        session_id: "s-micro-noop".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: Some(recent),
    };

    let descriptors = vec![compact_clearable_descriptor("readFile")];

    let bundle = ContextRuntime::new(100_000)
        .with_micro_compact_config(MicroCompactConfig {
            gap_threshold_secs: 3600,
            keep_recent_results: 0,
        })
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.micro_compact_stats.cleared_count, 0);
}

#[test]
fn micro_compact_then_persistence_budget_cooperation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let big_old = "x".repeat(30_000);
    let big_recent = "y".repeat(20_000);
    let small = "z".repeat(5_000);

    let messages = make_tool_conversation(&[
        ("readFile", "call-old", &big_old),
        ("readFile", "call-recent", &big_recent),
        ("readFile", "call-small", &small),
    ]);

    let past = chrono::Utc::now() - chrono::Duration::seconds(7200);
    let state = AgentState {
        session_id: "s-coop-test".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: Some(past),
    };

    let descriptors = vec![compact_clearable_descriptor("readFile")];

    let bundle = ContextRuntime::new(100_000)
        .with_micro_compact_config(MicroCompactConfig {
            gap_threshold_secs: 3600,
            keep_recent_results: 2,
        })
        .with_persistence_budget_config(PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 30_000,
        })
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert!(bundle.micro_compact_stats.cleared_count >= 1);
    assert_eq!(bundle.persistence_stats.persisted_count, 0);
}

#[test]
fn micro_compact_then_persistence_budget_still_persists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let big_old = "x".repeat(30_000);
    let big_recent = "y".repeat(40_000);
    let small = "z".repeat(5_000);

    let messages = make_tool_conversation(&[
        ("readFile", "call-old", &big_old),
        ("readFile", "call-recent", &big_recent),
        ("readFile", "call-small", &small),
    ]);

    let past = chrono::Utc::now() - chrono::Duration::seconds(7200);
    let state = AgentState {
        session_id: "s-coop-persist".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: Some(past),
    };

    let descriptors = vec![compact_clearable_descriptor("readFile")];

    let bundle = ContextRuntime::new(100_000)
        .with_micro_compact_config(MicroCompactConfig {
            gap_threshold_secs: 3600,
            keep_recent_results: 2,
        })
        .with_persistence_budget_config(PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 20_000,
        })
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert!(bundle.micro_compact_stats.cleared_count >= 1);
    assert!(bundle.persistence_stats.persisted_count >= 1);
}

#[test]
fn no_config_is_backward_compatible() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let big = "x".repeat(40_000);
    let messages = make_tool_conversation(&[
        ("readFile", "call-1", &big),
        ("readFile", "call-2", "small"),
    ]);

    let past = chrono::Utc::now() - chrono::Duration::seconds(7200);
    let state = AgentState {
        session_id: "s-compat".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: Some(past),
    };

    let bundle = ContextRuntime::new(100_000)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &[],
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.persistence_stats.persisted_count, 0);
    assert_eq!(bundle.micro_compact_stats.cleared_count, 0);
}

#[test]
fn deterministic_state_on_repeated_build_bundle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let big = "x".repeat(40_000);
    let mut messages = vec![LlmMessage::User {
        content: "read the file".to_string(),
        origin: UserMessageOrigin::User,
    }];
    messages.extend(make_tool_conversation(&[("shell", "call-1", &big)]));

    let state = AgentState {
        session_id: "s-deterministic".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: None,
    };

    let descriptors = vec![
        CapabilityDescriptor::builder("shell", CapabilityKind::tool())
            .description("test")
            .schema(json!({"type": "object"}), json!({"type": "string"}))
            .compact_clearable(false)
            .build()
            .expect("descriptor should build"),
    ];

    let runtime =
        ContextRuntime::new(100_000).with_persistence_budget_config(PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 10_000,
        });

    let bundle1 = runtime
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle1.persistence_stats.persisted_count, 1);

    let modified_content = bundle1
        .conversation
        .messages
        .iter()
        .find_map(|m| match m {
            LlmMessage::Tool {
                tool_call_id,
                content,
            } if tool_call_id == "call-1" => Some(content.clone()),
            _ => None,
        })
        .expect("should find call-1 in bundle output");

    if !modified_content.contains("<persisted-output>") {
        return;
    }

    let mut updated_messages = state.messages.clone();
    for msg in &mut updated_messages {
        if let LlmMessage::Tool {
            tool_call_id,
            content,
        } = msg
        {
            if tool_call_id == "call-1" {
                *content = modified_content.clone();
            }
        }
    }
    let state2 = AgentState {
        session_id: state.session_id.clone(),
        working_dir: state.working_dir.clone(),
        messages: updated_messages,
        phase: state.phase,
        turn_count: state.turn_count,
        last_assistant_at: state.last_assistant_at,
    };

    let bundle2 = runtime
        .build_bundle(
            &state2,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle2.persistence_stats.persisted_count, 0);
    assert!(bundle2.persistence_stats.skipped_already_persisted >= 1);
}

#[test]
fn full_pipeline_all_three_stages_active() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let big_old = "x".repeat(30_000);
    let big_recent = "y".repeat(40_000);
    let medium = "z".repeat(20_000);
    let small = "w".repeat(1_000);

    let messages = make_tool_conversation(&[
        ("readFile", "call-old", &big_old),
        ("readFile", "call-big", &big_recent),
        ("readFile", "call-medium", &medium),
        ("readFile", "call-small", &small),
    ]);

    let past = chrono::Utc::now() - chrono::Duration::seconds(7200);
    let state = AgentState {
        session_id: "s-full-pipeline".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: Some(past),
    };

    let descriptors = vec![compact_clearable_descriptor("readFile")];

    let bundle = ContextRuntime::new(10_000)
        .with_micro_compact_config(MicroCompactConfig {
            gap_threshold_secs: 3600,
            keep_recent_results: 2,
        })
        .with_persistence_budget_config(PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 5_000,
        })
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert!(bundle.micro_compact_stats.cleared_count >= 1);
    assert!(bundle.persistence_stats.persisted_count >= 1);
    assert!(
        bundle.prune_stats.truncated_tool_results >= 1
            || bundle.prune_stats.cleared_tool_results >= 1
    );
}

#[test]
fn updating_tool_result_max_bytes_keeps_other_context_stage_configs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let working = dir.path().join("project");
    std::fs::create_dir_all(&working).expect("create working dir");

    let messages = make_tool_conversation(&[
        ("readFile", "call-old", &"x".repeat(30_000)),
        ("readFile", "call-big", &"y".repeat(40_000)),
    ]);

    let state = AgentState {
        session_id: "s-preserve-config".to_string(),
        working_dir: working,
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: Some(chrono::Utc::now() - chrono::Duration::seconds(7200)),
    };

    let descriptors = vec![compact_clearable_descriptor("readFile")];

    let bundle = ContextRuntime::new(100_000)
        .with_persistence_budget_config(PersistenceBudgetConfig {
            aggregate_result_bytes_budget: 5_000,
        })
        .with_micro_compact_config(MicroCompactConfig {
            gap_threshold_secs: 3600,
            keep_recent_results: 1,
        })
        .with_tool_result_max_bytes(10_000)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 200_000,
            },
        )
        .expect("bundle should build");

    assert!(bundle.micro_compact_stats.cleared_count >= 1);
    assert!(bundle.persistence_stats.persisted_count >= 1);
}
