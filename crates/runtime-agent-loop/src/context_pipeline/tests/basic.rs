use std::sync::{Arc, Mutex};

use super::*;

fn make_state(messages: Vec<LlmMessage>) -> AgentState {
    AgentState {
        session_id: "session-1".to_string(),
        working_dir: std::env::temp_dir(),
        messages,
        phase: astrcode_core::Phase::Thinking,
        turn_count: 1,
        last_assistant_at: None,
    }
}

#[test]
fn default_runtime_materializes_baseline_messages() {
    let state = make_state(vec![LlmMessage::User {
        content: "hello".to_string(),
        origin: UserMessageOrigin::User,
    }]);

    let bundle = ContextRuntime::new(100_000)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &[],
                keep_recent_turns: 1,
                model_context_window: 8_192,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.conversation.messages.len(), state.messages.len());
    assert!(matches!(
        &bundle.conversation.messages[0],
        LlmMessage::User { content, .. } if content == "hello"
    ));
}

#[test]
fn compaction_view_stage_overrides_baseline_conversation() {
    let state = make_state(vec![LlmMessage::User {
        content: "old".to_string(),
        origin: UserMessageOrigin::User,
    }]);
    let compacted = CompactionView {
        messages: vec![LlmMessage::User {
            content: "summary".to_string(),
            origin: UserMessageOrigin::CompactSummary,
        }],
        memory_blocks: Vec::new(),
        recovery_refs: Vec::new(),
    };

    let bundle = ContextRuntime::new(100_000)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 1,
                prior_compaction_view: Some(&compacted),
                capability_descriptors: &[],
                keep_recent_turns: 1,
                model_context_window: 8_192,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.conversation.messages.len(), compacted.messages.len());
    assert!(matches!(
        &bundle.conversation.messages[0],
        LlmMessage::User { content, .. } if content == "summary"
    ));
}

#[test]
fn recovery_context_stage_injects_memory_blocks_and_refs() {
    let state = make_state(vec![LlmMessage::User {
        content: "old".to_string(),
        origin: UserMessageOrigin::User,
    }]);
    let compacted = CompactionView {
        messages: vec![LlmMessage::User {
            content: "summary".to_string(),
            origin: UserMessageOrigin::CompactSummary,
        }],
        memory_blocks: vec![ContextBlock {
            id: "recovered-file:src/lib.rs".to_string(),
            content: "fn recovered() {}".to_string(),
        }],
        recovery_refs: vec![RecoveryRef {
            kind: "file".to_string(),
            value: "src/lib.rs".to_string(),
        }],
    };

    let bundle = ContextRuntime::new(100_000)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 1,
                prior_compaction_view: Some(&compacted),
                capability_descriptors: &[],
                keep_recent_turns: 1,
                model_context_window: 8_192,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.memory.len(), 2);
    assert!(
        bundle
            .memory
            .iter()
            .any(|block| block.id == "recovered-file:src/lib.rs")
    );
    assert!(
        bundle
            .memory
            .iter()
            .any(|block| block.id == "recovery-refs" && block.content.contains("src/lib.rs"))
    );
}

struct RecordingStage {
    name: &'static str,
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl ContextStage for RecordingStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        _ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        self.order.lock().expect("order lock").push(self.name);
        bundle.diagnostics.push(ContextDiagnostic {
            stage: self.name,
            message: "visited".to_string(),
        });
        Ok(bundle)
    }
}

#[test]
fn custom_runtime_executes_stages_in_declared_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let runtime = ContextRuntime::from_stages(vec![
        Box::new(RecordingStage {
            name: "first",
            order: Arc::clone(&order),
        }),
        Box::new(RecordingStage {
            name: "second",
            order: Arc::clone(&order),
        }),
        Box::new(RecordingStage {
            name: "third",
            order: Arc::clone(&order),
        }),
    ]);

    let bundle = runtime
        .build_bundle(
            &make_state(Vec::new()),
            ContextBundleInput {
                turn_id: "turn-1",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &[],
                keep_recent_turns: 1,
                model_context_window: 8_192,
            },
        )
        .expect("bundle should build");

    assert_eq!(
        order.lock().expect("order lock").as_slice(),
        &["first", "second", "third"]
    );
    assert_eq!(bundle.diagnostics.len(), 3);
}

#[test]
fn default_runtime_keeps_structured_slots_alive() {
    let state = make_state(vec![LlmMessage::User {
        content: "hello".to_string(),
        origin: UserMessageOrigin::User,
    }]);

    let bundle = ContextRuntime::new(100_000)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-2",
                step_index: 7,
                prior_compaction_view: None,
                capability_descriptors: &[],
                keep_recent_turns: 1,
                model_context_window: 8_192,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.workset.len(), 1);
    assert!(
        bundle
            .diagnostics
            .iter()
            .any(|item| item.stage == "workset" && item.message.contains("step=7"))
    );
}

#[test]
fn tool_noise_trim_stage_runs_prune_pass_inside_pipeline() {
    let state = make_state(vec![
        LlmMessage::User {
            content: "inspect".to_string(),
            origin: UserMessageOrigin::User,
        },
        LlmMessage::Assistant {
            content: String::new(),
            tool_calls: vec![astrcode_core::ToolCallRequest {
                id: "call-1".to_string(),
                name: "readFile".to_string(),
                args: json!({"path":"Cargo.toml"}),
            }],
            reasoning: None,
        },
        LlmMessage::Tool {
            tool_call_id: "call-1".to_string(),
            content: "x".repeat(512),
        },
        LlmMessage::User {
            content: "follow up".to_string(),
            origin: UserMessageOrigin::User,
        },
    ]);
    let descriptors = vec![
        CapabilityDescriptor::builder("readFile", CapabilityKind::tool())
            .description("test")
            .schema(json!({"type":"object"}), json!({"type":"string"}))
            .compact_clearable(true)
            .build()
            .expect("descriptor should build"),
    ];

    let bundle = ContextRuntime::new(128)
        .build_bundle(
            &state,
            ContextBundleInput {
                turn_id: "turn-3",
                step_index: 0,
                prior_compaction_view: None,
                capability_descriptors: &descriptors,
                keep_recent_turns: 1,
                model_context_window: 8_192,
            },
        )
        .expect("bundle should build");

    assert_eq!(bundle.truncated_tool_results, 1);
    assert_eq!(bundle.prune_stats.cleared_tool_results, 1);
    assert!(matches!(
        &bundle.conversation.messages[2],
        LlmMessage::Tool { content, .. } if content.contains("[cleared older tool result")
    ));
}

#[test]
fn context_bundle_exports_memory_as_dynamic_prompt_declarations() {
    let bundle = ContextBundle {
        memory: vec![ContextBlock {
            id: "recovered-file:src/lib.rs".to_string(),
            content: "fn recovered() {}".to_string(),
        }],
        ..ContextBundle::default()
    };

    let declarations = bundle.prompt_declarations();

    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].layer, PromptLayer::Dynamic);
    assert_eq!(
        declarations[0].render_target,
        PromptDeclarationRenderTarget::System
    );
    assert!(
        declarations[0]
            .content
            .contains("recovered-file:src/lib.rs")
    );
    assert!(declarations[0].content.contains("fn recovered() {}"));
}

#[test]
fn context_bundle_clips_runtime_memory_prompt_declarations_deterministically() {
    let bundle = ContextBundle {
        memory: (0..6)
            .map(|index| ContextBlock {
                id: format!("recovered-file:file-{index}.rs"),
                content: format!("content-{index}-{}", "x".repeat(4_000)),
            })
            .collect(),
        ..ContextBundle::default()
    };

    let declarations = bundle.prompt_declarations();

    assert_eq!(declarations.len(), MAX_RUNTIME_MEMORY_PROMPT_BLOCKS);
    assert!(
        declarations
            .iter()
            .all(|declaration| !declaration.content.contains("file-0.rs")),
        "oldest runtime memory blocks should be trimmed first when prompt budget is exceeded"
    );
    assert!(
        declarations.iter().all(|declaration| {
            declaration.content.contains("file-2.rs")
                || declaration.content.contains("file-3.rs")
                || declaration.content.contains("file-4.rs")
                || declaration.content.contains("file-5.rs")
        }),
        "deterministic trimming should keep the newest runtime memory blocks under budget"
    );
}
