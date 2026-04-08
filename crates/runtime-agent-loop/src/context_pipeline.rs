//! # Context Pipeline（上下文管道）
//!
//! ## 职责
//!
//! 通过可组合的 pipeline stages 构建模型可见的上下文包（`ContextBundle`）。
//! 将"可用的材料是什么"与"循环何时使用它"严格分离，各阶段只做同步纯变换。
//!
//! ## 在 Turn 流程中的作用
//!
//! - **调用时机**：每个 step 开始时，`turn_runner` 调用 `build_bundle()`
//! - **输入**：`AgentState` + `ContextBundleInput`（turn_id/step_index/compaction view/模型窗口等）
//! - **输出**：`ContextBundle`（包含 `ConversationView`、workset、memory、诊断信息）
//! - **不变约束**：stage 不做 IO、不发事件、不做审批、不触发 compact
//!
//! ## Pipeline Stages（按执行顺序）
//!
//! | Stage | 职责 |
//! |-------|------|
//! | `BaselineStage` | 将 `AgentState.messages` 物化为初始对话视图 |
//! | `RecentTailStage` | 占位，当前 `AgentState.messages` 已包含完整尾部视图 |
//! | `WorksetStage` | 注入工作目录等结构化工作集槽位 |
//! | `CompactionViewStage` | 若已有压缩后的窄视图，覆盖 baseline 对话视图 |
//! | `RecoveryContextStage` | 注入 compact rebuild 产出的瞬时恢复上下文 |
//! | `PrunePassStage` | 运行 prune pass，裁剪大工具结果、清理安全工具元数据 |
//! | `BudgetTrimStage` | 占位，未来实现 token budget 感知的主动裁剪 |
//!
//! ## 依赖和协作
//!
//! - **使用** `apply_prune_pass` / `effective_context_window` 执行本地 prune
//! - **被调用方**：`turn_runner` 在每个 step 中调用 `build_bundle()`
//! - **输出给**：`PromptRuntime.build_plan()` 和 `RequestAssembler` 消费 `ContextBundle`
//! - **与 Compaction 的关系**：Compaction 重建 `CompactionView` 后，通过 `prior_compaction_view`
//!   传入管道，`CompactionViewStage` / `RecoveryContextStage` 负责将其注入到 bundle 中
//!
//! ## 关键设计
//!
//! - `tool_result_max_bytes()` 暴露给 `AgentLoop`，供 `RuntimeService` 装配时查询
//! - `ContextBundleInput` 结构体收口了 6 个 per-step 参数，避免函数参数膨胀

use std::path::Path;

use astrcode_core::{AgentState, LlmMessage, Result};
use astrcode_protocol::capability::CapabilityDescriptor;

use crate::context_window::{PruneStats, apply_prune_pass, effective_context_window};

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

/// compact rebuild 后的窄视图。
///
/// 这里把可继续对话的 messages 与瞬时恢复上下文分开，避免再把恢复文件内容
/// 伪装成 compact summary 混回消息流。
#[derive(Debug, Clone, Default)]
pub(crate) struct CompactionView {
    pub messages: Vec<LlmMessage>,
    pub memory_blocks: Vec<ContextBlock>,
    pub recovery_refs: Vec<RecoveryRef>,
}

/// compact rebuild 产出的恢复引用。
///
/// 首期主要承接最近关键文件等可恢复材料，后续可以继续扩展到 ghost snapshot
/// 或 compact metadata，而不必再次拆结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecoveryRef {
    pub kind: String,
    pub value: String,
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
    pub prune_stats: PruneStats,
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
    pub prior_compaction_view: Option<&'a CompactionView>,
    pub capability_descriptors: &'a [CapabilityDescriptor],
    pub keep_recent_turns: usize,
    pub model_context_window: usize,
    pub tool_result_max_bytes: usize,
}

/// 组装模型可见上下文包时所需的 per-step 输入。
///
/// 这样 `build_bundle` 只接受一个语义化参数，避免 turn runner 继续把一串相关值手工
/// 展开传递给 pipeline。
pub(crate) struct ContextBundleInput<'a> {
    pub turn_id: &'a str,
    pub step_index: usize,
    pub prior_compaction_view: Option<&'a CompactionView>,
    pub capability_descriptors: &'a [CapabilityDescriptor],
    pub keep_recent_turns: usize,
    pub model_context_window: usize,
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
                Box::new(RecoveryContextStage),
                Box::new(PrunePassStage),
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
        input: ContextBundleInput<'_>,
    ) -> Result<ContextBundle> {
        let ctx = ContextStageContext {
            session_id: &state.session_id,
            working_dir: &state.working_dir,
            turn_id: input.turn_id,
            step_index: input.step_index,
            base_messages: &state.messages,
            prior_compaction_view: input.prior_compaction_view,
            capability_descriptors: input.capability_descriptors,
            keep_recent_turns: input.keep_recent_turns,
            model_context_window: input.model_context_window,
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
            bundle.conversation = ConversationView::new(view.messages.clone());
        }
        Ok(bundle)
    }
}

/// 注入 compact rebuild 产出的恢复上下文。
///
/// 把恢复块和恢复引用折叠进 bundle.memory，而不是混进 conversation messages。
struct RecoveryContextStage;

impl ContextStage for RecoveryContextStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        let Some(view) = ctx.prior_compaction_view else {
            return Ok(bundle);
        };

        bundle.memory.extend(view.memory_blocks.iter().cloned());
        if !view.recovery_refs.is_empty() {
            let refs = view
                .recovery_refs
                .iter()
                .map(|item| format!("- [{}] {}", item.kind, item.value))
                .collect::<Vec<_>>()
                .join("\n");
            bundle.memory.push(ContextBlock {
                id: "recovery-refs".to_string(),
                content: format!("Recent recovery refs:\n{refs}"),
            });
        }
        Ok(bundle)
    }
}

/// 直接在模型可见的 conversation 上执行本地 prune。
///
/// 这样 request assembler 继续保持纯编码边界，不再悄悄修改上下文内容。
struct PrunePassStage;

impl ContextStage for PrunePassStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        let result = apply_prune_pass(
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
        bundle.truncated_tool_results = result.stats.truncated_tool_results;
        bundle.prune_stats = result.stats;
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

    use astrcode_core::{LlmMessage, UserMessageOrigin};
    use astrcode_protocol::capability::{CapabilityDescriptor, CapabilityKind};
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
}
