use super::*;

fn test_compact_config() -> CompactConfig {
    CompactConfig {
        keep_recent_turns: 1,
        keep_recent_user_messages: 8,
        trigger: astrcode_core::CompactTrigger::Manual,
        summary_reserve_tokens: 20_000,
        max_output_tokens: 20_000,
        max_retry_attempts: 3,
        history_path: None,
        custom_instructions: None,
    }
}

#[test]
fn render_compact_system_prompt_keeps_do_not_continue_instruction_intact() {
    let prompt =
        render_compact_system_prompt(None, CompactPromptMode::Fresh, 20_000, &[], None, None);

    assert!(
        prompt.contains("**Do NOT continue the conversation.**"),
        "compact prompt must explicitly instruct the summarizer not to continue the session"
    );
}

#[test]
fn render_compact_system_prompt_renders_incremental_block() {
    let prompt = render_compact_system_prompt(
        None,
        CompactPromptMode::Incremental {
            previous_summary: "older summary".to_string(),
        },
        20_000,
        &[],
        None,
        None,
    );

    assert!(prompt.contains("## Incremental Mode"));
    assert!(prompt.contains("<previous-summary>"));
    assert!(prompt.contains("older summary"));
}

#[test]
fn render_compact_system_prompt_includes_output_cap_and_recent_user_context_messages() {
    let prompt = render_compact_system_prompt(
        None,
        CompactPromptMode::Fresh,
        12_345,
        &[RecentUserContextMessage {
            index: 7,
            content: "保留这条约束".to_string(),
        }],
        None,
        None,
    );

    assert!(prompt.contains("12345"));
    assert!(prompt.contains("Recently Preserved Real User Messages"));
    assert!(prompt.contains("保留这条约束"));
    assert!(prompt.contains("<recent_user_context_digest>"));
}

#[test]
fn render_compact_system_prompt_includes_contract_repair_feedback() {
    let prompt = render_compact_system_prompt(
        None,
        CompactPromptMode::Fresh,
        12_345,
        &[],
        None,
        Some("missing <recent_user_context_digest>"),
    );

    assert!(prompt.contains("## Contract Repair"));
    assert!(prompt.contains("missing <recent_user_context_digest>"));
}

#[test]
fn merge_compact_prompt_context_appends_hook_suffix_after_runtime_prompt() {
    let merged = merge_compact_prompt_context(Some("base"), Some("hook"))
        .expect("merged compact prompt context should exist");

    assert_eq!(merged, "base\n\nhook");
}

#[test]
fn merge_compact_prompt_context_returns_none_when_both_empty() {
    assert!(merge_compact_prompt_context(None, None).is_none());
    assert!(merge_compact_prompt_context(Some("   "), Some(" \n\t ")).is_none());
}

#[test]
fn parse_compact_output_requires_non_empty_content() {
    let error = parse_compact_output("   ").expect_err("empty compact output should fail");
    assert!(error.to_string().contains("missing <summary> block"));
}

#[test]
fn parse_compact_output_requires_closed_summary_block() {
    let error = parse_compact_output("<summary>open").expect_err("unclosed summary should fail");
    assert!(error.to_string().contains("closing </summary>"));
}

#[test]
fn parse_compact_output_prefers_summary_block() {
    let parsed = parse_compact_output(
        "<analysis>draft</analysis><summary>\nSection\n</\
         summary><recent_user_context_digest>(none)</recent_user_context_digest>",
    )
    .expect("summary should parse");

    assert_eq!(parsed.summary, "Section");
    assert_eq!(parsed.recent_user_context_digest.as_deref(), Some("(none)"));
    assert!(parsed.has_analysis);
    assert!(parsed.has_recent_user_context_digest_block);
}

#[test]
fn parse_compact_output_accepts_case_insensitive_summary_block() {
    let parsed = parse_compact_output(
        "<ANALYSIS>draft</ANALYSIS><SUMMARY>Section</SUMMARY><RECENT_USER_CONTEXT_DIGEST>digest</\
         RECENT_USER_CONTEXT_DIGEST>",
    )
    .expect("summary should parse");

    assert_eq!(parsed.summary, "Section");
    assert_eq!(parsed.recent_user_context_digest.as_deref(), Some("digest"));
    assert!(parsed.has_analysis);
    assert!(parsed.has_recent_user_context_digest_block);
}

#[test]
fn parse_compact_output_falls_back_to_plain_text_summary() {
    let parsed = parse_compact_output("## Goal\n- preserve current task")
        .expect("plain text summary should parse");

    assert_eq!(parsed.summary, "## Goal\n- preserve current task");
    assert!(!parsed.has_analysis);
    assert!(!parsed.has_recent_user_context_digest_block);
}

#[test]
fn parse_compact_output_strips_outer_code_fence_before_parsing() {
    let parsed =
        parse_compact_output("```xml\n<analysis>draft</analysis>\n<summary>Section</summary>\n```")
            .expect("fenced xml summary should parse");

    assert_eq!(parsed.summary, "Section");
    assert!(parsed.has_analysis);
    assert!(!parsed.has_recent_user_context_digest_block);
}

#[test]
fn compact_contract_violation_flags_missing_digest_block() {
    let violation = CompactContractViolation::from_parsed_output(&ParsedCompactOutput {
        summary: "Section".to_string(),
        recent_user_context_digest: None,
        has_analysis: true,
        has_recent_user_context_digest_block: false,
        used_fallback: false,
    })
    .expect("missing digest block should violate contract");

    assert!(violation.detail.contains("recent_user_context_digest"));
}

#[test]
fn parse_compact_output_strips_common_summary_preamble_in_fallback() {
    let parsed = parse_compact_output("Summary:\n## Goal\n- preserve current task")
        .expect("summary preamble fallback should parse");

    assert_eq!(parsed.summary, "## Goal\n- preserve current task");
}

#[test]
fn parse_compact_output_accepts_summary_tag_attributes() {
    let parsed = parse_compact_output(
        "<analysis class=\"draft\">draft</analysis><summary format=\"markdown\">Section</summary>",
    )
    .expect("tag attributes should parse");

    assert_eq!(parsed.summary, "Section");
    assert!(parsed.has_analysis);
}

#[test]
fn parse_compact_output_does_not_treat_analysis_only_as_summary() {
    let error = parse_compact_output("<analysis>draft</analysis>")
        .expect_err("analysis-only output should still fail");

    assert!(error.to_string().contains("missing <summary> block"));
}

#[test]
fn split_for_compaction_preserves_recent_real_user_turns() {
    let messages = vec![
        LlmMessage::User {
            content: "older".to_string(),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: "ack".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
        LlmMessage::User {
            content: format_compact_summary("older"),
            origin: UserMessageOrigin::CompactSummary,
        },
        LlmMessage::User {
            content: "newer".to_string(),
            origin: UserMessageOrigin::User,
        },
    ];

    let split = split_for_compaction(&messages, 1).expect("split should exist");

    assert_eq!(split.keep_start, 3);
    assert_eq!(split.prefix.len(), 3);
    assert_eq!(split.suffix.len(), 1);
}

#[test]
fn split_for_compaction_falls_back_to_assistant_boundary_for_single_turn() {
    let messages = vec![
        LlmMessage::User {
            content: "task".to_string(),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: "step 1".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
        LlmMessage::Assistant {
            content: "step 2".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
    ];

    let split = split_for_compaction(&messages, 1).expect("single turn should still split");
    assert_eq!(split.keep_start, 2);
}

#[test]
fn compacted_messages_inserts_summary_as_compact_user_message() {
    let compacted = compacted_messages("Older history", None, &[], 0, Vec::new());

    assert!(matches!(
        &compacted[0],
        LlmMessage::User {
            origin: UserMessageOrigin::CompactSummary,
            ..
        }
    ));
    assert_eq!(compacted.len(), 1);
}

#[test]
fn prepare_compact_input_strips_history_note_from_previous_summary() {
    let filtered = prepare_compact_input(&[LlmMessage::User {
        content: CompactSummaryEnvelope::new("older summary")
            .with_history_path("~/.astrcode/projects/demo/sessions/abc/session-abc.jsonl")
            .render(),
        origin: UserMessageOrigin::CompactSummary,
    }]);

    assert!(matches!(
        filtered.prompt_mode,
        CompactPromptMode::Incremental { ref previous_summary }
            if previous_summary == "older summary"
    ));
}

#[test]
fn prepare_compact_input_skips_synthetic_user_messages() {
    let filtered = prepare_compact_input(&[
        LlmMessage::User {
            content: "summary".to_string(),
            origin: UserMessageOrigin::CompactSummary,
        },
        LlmMessage::User {
            content: "wake up".to_string(),
            origin: UserMessageOrigin::ReactivationPrompt,
        },
        LlmMessage::User {
            content: "digest".to_string(),
            origin: UserMessageOrigin::RecentUserContextDigest,
        },
        LlmMessage::User {
            content: "preserved".to_string(),
            origin: UserMessageOrigin::RecentUserContext,
        },
        LlmMessage::User {
            content: "real user".to_string(),
            origin: UserMessageOrigin::User,
        },
    ]);

    assert_eq!(filtered.messages.len(), 1);
    assert!(matches!(
        &filtered.messages[0],
        LlmMessage::User {
            content,
            origin: UserMessageOrigin::User
        } if content == "real user"
    ));
}

#[test]
fn build_compact_result_marks_incremental_mode_when_previous_summary_exists() {
    let prepared_input = prepare_compact_input(&[
        LlmMessage::User {
            content: CompactSummaryEnvelope::new("older summary").render(),
            origin: UserMessageOrigin::CompactSummary,
        },
        LlmMessage::User {
            content: "current task".to_string(),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: "latest step".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
    ]);

    let result = build_compact_result(
        CompactResultInput {
            compacted_messages: compacted_messages(
                "refreshed summary",
                Some("- keep current objective"),
                &[],
                2,
                vec![LlmMessage::Assistant {
                    content: "latest step".to_string(),
                    tool_calls: Vec::new(),
                    reasoning: None,
                }],
            ),
            summary: "refreshed summary".to_string(),
            recent_user_context_digest: Some("- keep current objective".to_string()),
            recent_user_context_messages: Vec::new(),
            preserved_recent_turns: 1,
            pre_tokens: 256,
            messages_removed: 2,
        },
        None,
        &test_compact_config(),
        CompactExecutionResult {
            parsed_output: ParsedCompactOutput {
                summary: "refreshed summary".to_string(),
                recent_user_context_digest: Some("- keep current objective".to_string()),
                has_analysis: true,
                has_recent_user_context_digest_block: true,
                used_fallback: false,
            },
            prepared_input,
            retry_state: CompactRetryState::default(),
        },
    );

    assert_eq!(result.meta.mode, CompactMode::Incremental);
    assert_eq!(result.meta.retry_count, 0);
    assert!(!result.meta.fallback_used);
    assert_eq!(result.meta.input_units, 2);
}

#[test]
fn normalize_compaction_tool_content_removes_exact_child_identifiers() {
    let normalized = normalize_compaction_tool_content(
        "spawn 已在后台启动。\n\nChild agent reference:\n- agentId: agent-1\n- subRunId: \
         subrun-1\n- sessionId: session-parent\n- openSessionId: session-child\n- status: \
         running\nUse this exact `agentId` value in later send/observe/close calls.",
    );

    assert!(normalized.contains("spawn 已在后台启动。"));
    assert!(normalized.contains("Do not reuse any agentId"));
    assert!(!normalized.contains("agent-1"));
    assert!(!normalized.contains("subrun-1"));
    assert!(!normalized.contains("session-child"));
}


#[test]
fn sanitize_compact_summary_replaces_stale_route_identifiers_with_boundary_guidance() {
    let sanitized = sanitize_compact_summary(
        "## Progress\n- Spawned agent-3 and later called observe(agent-2).\n- Error: agent \
         'agent-2' is not a direct child of caller 'agent-root:session-parent' (actual parent: \
         agent-1); send/observe/close only support direct children.\n- Child ref payload: \
         agentId=agent-2 subRunId=subrun-2 openSessionId=session-child-2",
    );

    assert!(sanitized.contains("## Compact Boundary"));
    assert!(sanitized.contains("live direct-child snapshot"));
    assert!(sanitized.contains("<agent-id>"));
    assert!(sanitized.contains("<subrun-id>") || sanitized.contains("<direct-child-subRunId>"));
    assert!(sanitized.contains("<child-session-id>") || sanitized.contains("<session-id>"));
    assert!(!sanitized.contains("agent-2"));
    assert!(!sanitized.contains("subrun-2"));
    assert!(!sanitized.contains("session-child-2"));
    assert!(!sanitized.contains("not a direct child of caller"));
}

#[test]
fn drop_oldest_compaction_unit_is_deterministic() {
    let mut prefix = vec![
        LlmMessage::User {
            content: "task".to_string(),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: "step-1".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
        LlmMessage::Assistant {
            content: "step-2".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
    ];

    assert!(drop_oldest_compaction_unit(&mut prefix));
    assert!(matches!(
        &prefix[0],
        LlmMessage::Assistant { content, .. } if content == "step-1"
    ));
}

#[test]
fn trim_prefix_until_compact_request_fits_drops_oldest_units_before_calling_llm() {
    let mut prefix = vec![
        LlmMessage::User {
            content: "very old request ".repeat(1200),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: "first step".repeat(1200),
            tool_calls: Vec::new(),
            reasoning: None,
        },
        LlmMessage::Assistant {
            content: "latest step".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
    ];

    let trimmed = trim_prefix_until_compact_request_fits(
        &mut prefix,
        None,
        ModelLimits {
            context_window: 23_000,
            max_output_tokens: 2_000,
        },
        &test_compact_config(),
        &[],
    );

    assert!(trimmed);
    assert!(matches!(
        prefix.as_slice(),
        [LlmMessage::Assistant { content, .. }] if content == "latest step"
    ));
}

#[test]
fn can_compact_returns_false_for_empty_messages() {
    assert!(!can_compact(&[], 2));
}

#[test]
fn can_compact_returns_true_when_enough_turns() {
    let messages = vec![
        LlmMessage::User {
            content: "turn-1".to_string(),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: "reply".to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
        LlmMessage::User {
            content: "turn-2".to_string(),
            origin: UserMessageOrigin::User,
        },
    ];
    assert!(can_compact(&messages, 1));
}

#[test]
fn collect_recent_user_context_messages_only_keeps_real_users() {
    let recent = collect_recent_user_context_messages(
        &[
            LlmMessage::User {
                content: "summary".to_string(),
                origin: UserMessageOrigin::CompactSummary,
            },
            LlmMessage::User {
                content: "recent".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::User {
                content: "digest".to_string(),
                origin: UserMessageOrigin::RecentUserContextDigest,
            },
            LlmMessage::User {
                content: "latest".to_string(),
                origin: UserMessageOrigin::User,
            },
        ],
        8,
    );

    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].content, "recent");
    assert_eq!(recent[1].content, "latest");
}

#[test]
fn compacted_messages_put_recent_user_context_before_suffix_without_duplicates() {
    let messages = compacted_messages(
        "Older history",
        Some("- keep current objective"),
        &[RecentUserContextMessage {
            index: 1,
            content: "latest user".to_string(),
        }],
        1,
        vec![
            LlmMessage::User {
                content: "latest user".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "latest assistant".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
        ],
    );

    assert!(matches!(
        &messages[0],
        LlmMessage::User {
            origin: UserMessageOrigin::CompactSummary,
            ..
        }
    ));
    assert!(matches!(
        &messages[1],
        LlmMessage::User {
            origin: UserMessageOrigin::RecentUserContextDigest,
            ..
        }
    ));
    assert!(matches!(
        &messages[2],
        LlmMessage::User {
            origin: UserMessageOrigin::RecentUserContext,
            content,
        } if content == "latest user"
    ));
    assert!(matches!(
        &messages[3],
        LlmMessage::Assistant { content, .. } if content == "latest assistant"
    ));
    assert_eq!(messages.len(), 4);
}
