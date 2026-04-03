//! 用于构建模型可见会话包的上下文管道
//!
//! 该管道有意保持简洁：各阶段仅将只读快照转换为`ContextBundle`
//! 它们不会与提供者交互、触发事件或决定循环是否应压缩/重试
//! 这将“可用的材料是什么”与“循环何时使用它”分开

use std::path::Path;

use astrcode_core::{AgentState, CapabilityDescriptor, LlmMessage, Result};

use crate::context_window::{apply_microcompact, effective_context_window};

/// 当前对模型可见的会话视图。
///
/// 这不是完整历史，也不是 event log replay 的直接结果，而是当前 turn 在模型侧可见的
/// 会话材料。把它单独抽出来，是为了让 compact 只重建会话视图，不重新编排整个上下文包。
#[derive(Debug, Clone, Default)]
pub(crate) struct ConversationView {
    pub messages: Vec<LlmMessage>,
}

impl ConversationView {
    pub(crate) fn new(messages: Vec<LlmMessage>) -> Self {
        Self { messages }
    }
}

/// 结构化的上下文块占位。
///
/// 本轮先把 workset/memory 作为显式槽位保留下来，后续阶段可以在不改 loop 主干的情况下
/// 补充更细的工作集/记忆来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextBlock {
    pub id: String,
    pub content: String,
}

/// 上下文诊断信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextDiagnostic {
    pub stage: &'static str,
    pub message: String,
}

/// Token budget 在上下文层的轻量占位。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TokenBudgetState;

/// Context pipeline 的中间结果。
#[derive(Debug, Clone, Default)]
pub(crate) struct ContextBundle {
    pub conversation: ConversationView,
    pub workset: Vec<ContextBlock>,
    pub memory: Vec<ContextBlock>,
    pub diagnostics: Vec<ContextDiagnostic>,
    pub budget_state: TokenBudgetState,
    pub truncated_tool_results: usize,
}

/// Stage 的只读输入快照。
///
/// 这里刻意只暴露已准备好的材料和基础元数据，避免 stage 重新去读 session 真相或触发副作用。
pub(crate) struct ContextStageContext<'a> {
    pub session_id: &'a str,
    pub working_dir: &'a Path,
    pub turn_id: &'a str,
    pub step_index: usize,
    pub base_messages: &'a [LlmMessage],
    pub prior_compaction_view: Option<&'a ConversationView>,
    pub capability_descriptors: &'a [CapabilityDescriptor],
    pub keep_recent_turns: usize,
    pub model_context_window: usize,
    pub tool_result_max_bytes: usize,
}

/// Pipeline stage。
///
/// 约束：stage 只做同步纯变换，不做 IO、不发事件、不做审批，也不触发 compact。
pub(crate) trait ContextStage: Send + Sync {
    fn apply(&self, bundle: ContextBundle, ctx: &ContextStageContext<'_>) -> Result<ContextBundle>;
}

/// Runtime-facing pipeline wrapper.
pub(crate) struct ContextRuntime {
    stages: Vec<Box<dyn ContextStage>>,
    tool_result_max_bytes: usize,
}

impl ContextRuntime {
    pub fn new(tool_result_max_bytes: usize) -> Self {
        Self {
            stages: vec![
                Box::new(BaselineStage),
                Box::new(RecentTailStage),
                Box::new(WorksetStage),
                Box::new(CompactionViewStage),
                Box::new(ToolNoiseTrimStage),
                Box::new(BudgetTrimStage),
            ],
            tool_result_max_bytes: tool_result_max_bytes.max(1),
        }
    }

    #[cfg(test)]
    fn from_stages(stages: Vec<Box<dyn ContextStage>>) -> Self {
        Self {
            stages,
            tool_result_max_bytes: 100_000, // 测试默认值，与 runtime-config 保持一致
        }
    }

    pub(crate) fn tool_result_max_bytes(&self) -> usize {
        self.tool_result_max_bytes
    }

    /// Build the model-visible context bundle from readonly loop inputs.
    pub(crate) fn build_bundle(
        &self,
        state: &AgentState,
        turn_id: &str,
        step_index: usize,
        prior_compaction_view: Option<&ConversationView>,
        capability_descriptors: &[CapabilityDescriptor],
        keep_recent_turns: usize,
        model_context_window: usize,
    ) -> Result<ContextBundle> {
        let ctx = ContextStageContext {
            session_id: &state.session_id,
            working_dir: &state.working_dir,
            turn_id,
            step_index,
            base_messages: &state.messages,
            prior_compaction_view,
            capability_descriptors,
            keep_recent_turns,
            model_context_window,
            tool_result_max_bytes: self.tool_result_max_bytes,
        };
        let mut bundle = ContextBundle::default();
        for stage in &self.stages {
            bundle = stage.apply(bundle, &ctx)?;
        }
        Ok(bundle)
    }
}

/// Materialize the baseline projected messages.
struct BaselineStage;

impl ContextStage for BaselineStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        if bundle.conversation.messages.is_empty() {
            bundle.conversation = ConversationView::new(ctx.base_messages.to_vec());
        }
        Ok(bundle)
    }
}

/// Placeholder for future tail-focused materialization.
///
/// The current projected `AgentState.messages` already carries the full tail view, so this stage is
/// intentionally a no-op until the runtime starts exposing finer-grained stored-event snapshots.
struct RecentTailStage;

impl ContextStage for RecentTailStage {
    fn apply(
        &self,
        bundle: ContextBundle,
        _ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        Ok(bundle)
    }
}

/// Placeholder for future workset injection.
struct WorksetStage;

impl ContextStage for WorksetStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        // Keep a minimal structured workset slot alive from day one so later phases can enrich it
        // without changing the bundle shape that the loop already depends on.
        bundle.workset.push(ContextBlock {
            id: "working-dir".to_string(),
            content: ctx.working_dir.to_string_lossy().into_owned(),
        });
        bundle.diagnostics.push(ContextDiagnostic {
            stage: "workset",
            message: format!(
                "session={} turn={} step={}",
                ctx.session_id, ctx.turn_id, ctx.step_index
            ),
        });
        Ok(bundle)
    }
}

/// Override the conversation view when compaction already rebuilt a narrower model-visible view.
struct CompactionViewStage;

impl ContextStage for CompactionViewStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        if let Some(view) = ctx.prior_compaction_view {
            bundle.conversation = view.clone();
        }
        Ok(bundle)
    }
}

/// Trim tool noise directly on the model-visible conversation view.
///
/// Microcompact now lives in the pipeline so request assembly stays a pure encoding step. That
/// keeps tool-result pruning in the same place as the rest of context material selection instead
/// of letting `RequestAssembler` mutate the conversation after the fact.
struct ToolNoiseTrimStage;

impl ContextStage for ToolNoiseTrimStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        let result = apply_microcompact(
            &bundle.conversation.messages,
            ctx.capability_descriptors,
            ctx.tool_result_max_bytes,
            ctx.keep_recent_turns,
            effective_context_window(astrcode_runtime_llm::ModelLimits {
                context_window: ctx.model_context_window,
                max_output_tokens: 0,
            }),
        );
        bundle.conversation = ConversationView::new(result.messages);
        bundle.truncated_tool_results = result.truncated_tool_results;
        Ok(bundle)
    }
}

/// Placeholder for future token-budget-aware trimming.
///
/// Token budget tracking currently lives in `TokenUsageTracker` and influences
/// the auto-continue nudge logic. This stage will eventually decide whether to
/// proactively trim the conversation view when approaching the user-specified budget.
struct BudgetTrimStage;

impl ContextStage for BudgetTrimStage {
    fn apply(
        &self,
        bundle: ContextBundle,
        _ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        Ok(bundle)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{CapabilityKind, LlmMessage, UserMessageOrigin};
    use serde_json::json;

    use super::*;

    fn make_state(messages: Vec<LlmMessage>) -> AgentState {
        AgentState {
            session_id: "session-1".to_string(),
            working_dir: std::env::temp_dir(),
            messages,
            phase: astrcode_core::Phase::Thinking,
            turn_count: 1,
        }
    }

    #[test]
    fn default_runtime_materializes_baseline_messages() {
        let state = make_state(vec![LlmMessage::User {
            content: "hello".to_string(),
            origin: UserMessageOrigin::User,
        }]);

        let bundle = ContextRuntime::new(100_000)
            .build_bundle(&state, "turn-1", 0, None, &[], 1, 8_192)
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
        let compacted = ConversationView::new(vec![LlmMessage::User {
            content: "summary".to_string(),
            origin: UserMessageOrigin::CompactSummary,
        }]);

        let bundle = ContextRuntime::new(100_000)
            .build_bundle(&state, "turn-1", 1, Some(&compacted), &[], 1, 8_192)
            .expect("bundle should build");

        assert_eq!(bundle.conversation.messages.len(), compacted.messages.len());
        assert!(matches!(
            &bundle.conversation.messages[0],
            LlmMessage::User { content, .. } if content == "summary"
        ));
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
            .build_bundle(&make_state(Vec::new()), "turn-1", 0, None, &[], 1, 8_192)
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
            .build_bundle(&state, "turn-2", 7, None, &[], 1, 8_192)
            .expect("bundle should build");

        assert_eq!(bundle.workset.len(), 1);
        assert!(bundle
            .diagnostics
            .iter()
            .any(|item| item.stage == "workset" && item.message.contains("step=7")));
    }

    #[test]
    fn tool_noise_trim_stage_runs_microcompact_inside_pipeline() {
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
        let descriptors =
            vec![
                astrcode_core::CapabilityDescriptor::builder("readFile", CapabilityKind::tool())
                    .description("test")
                    .schema(json!({"type":"object"}), json!({"type":"string"}))
                    .compact_clearable(true)
                    .build()
                    .expect("descriptor should build"),
            ];

        let bundle = ContextRuntime::new(128)
            .build_bundle(&state, "turn-3", 0, None, &descriptors, 1, 8_192)
            .expect("bundle should build");

        assert_eq!(bundle.truncated_tool_results, 1);
        assert!(matches!(
            &bundle.conversation.messages[2],
            LlmMessage::Tool { content, .. } if content.contains("[cleared older tool result")
        ));
    }
}
