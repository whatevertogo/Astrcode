//! # Context Pipeline（上下文管道）
//!
//! ## 职责
//!
//! 通过可组合的 pipeline stages 构建模型可见的上下文包（`ContextBundle`）。
//! 将"可用的材料是什么"与"循环何时使用它"严格分离，各阶段以同步纯变换为主。
//!
//! **例外**：`PersistenceBudgetStage` 被正式授权做受控文件 IO（同步、幂等、
//! 失败降级），但只修改 `ContextBundle`，不修改 `AgentState` 或事件日志。
//! 详见该 stage 的文档。
//!
//! ## 在 Turn 流程中的作用
//!
//! - **调用时机**：每个 step 开始时，`turn_runner` 调用 `build_bundle()`
//! - **输入**：`AgentState` + `ContextBundleInput`（turn_id/step_index/compaction view/模型窗口等）
//! - **输出**：`ContextBundle`（包含 `ConversationView`、workset、memory、诊断信息）
//! - **不变约束**：stage 以同步纯变换为主，不发事件、不做审批、不触发 compact。
//!   唯一例外：`PersistenceBudgetStage` 可做受控 IO（同步幂等、失败降级）。
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
//! | `MicroCompactStage` | 空闲超阈值时清除旧可压缩工具结果 |
//! | `PersistenceBudgetStage` | 聚合预算超限时强制落盘最大工具结果（受控 IO） |
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

use std::path::{Path, PathBuf};

use astrcode_core::{AgentState, LlmMessage, Result};
use astrcode_protocol::capability::CapabilityDescriptor;
use astrcode_runtime_prompt::{
    PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, PromptLayer,
};

use crate::context_window::{
    PersistenceStats, PruneStats, apply_prune_pass, effective_context_window,
    micro_compact::{MicroCompactConfig, MicroCompactStats, apply_micro_compact, should_trigger},
    tool_result_persistence::{PersistenceBudgetConfig, enforce_aggregate_budget},
};

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
    pub persistence_stats: PersistenceStats,
    pub micro_compact_stats: MicroCompactStats,
}

const MAX_RUNTIME_MEMORY_PROMPT_BLOCKS: usize = 4;

impl ContextBundle {
    /// 将运行时 memory 槽位转换成 prompt declaration。
    ///
    /// memory 块属于瞬时恢复材料，应该进入 system prompt，而不是污染消息流或 durable transcript。
    pub(crate) fn prompt_declarations(&self) -> Vec<PromptDeclaration> {
        let _reserved_runtime_slots = (&self.workset, &self.diagnostics, &self.budget_state);
        let mut declarations = self
            .memory
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                let content = block.content.trim();
                if content.is_empty() {
                    return None;
                }
                Some(PromptDeclaration {
                    block_id: format!("runtime.memory.{index}"),
                    title: "Recovered Context".to_string(),
                    content: format!("Source: {}\n{}", block.id, content),
                    render_target: PromptDeclarationRenderTarget::System,
                    layer: PromptLayer::Dynamic,
                    kind: PromptDeclarationKind::ExtensionInstruction,
                    priority_hint: Some(880),
                    always_include: true,
                    source: PromptDeclarationSource::Builtin,
                    capability_name: None,
                    origin: Some(format!("runtime-memory:{}", block.id)),
                })
            })
            .fold(Vec::new(), |mut declarations, declaration| {
                if declarations
                    .last()
                    .is_some_and(|previous: &PromptDeclaration| {
                        previous.origin == declaration.origin
                            && previous.content == declaration.content
                    })
                {
                    return declarations;
                }
                declarations.push(declaration);
                declarations
            });

        while declarations.len() > MAX_RUNTIME_MEMORY_PROMPT_BLOCKS {
            declarations.remove(0);
        }

        declarations
    }
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
    /// 会话目录路径，供 PersistenceBudgetStage 做受控 IO 使用。
    /// None 时 PersistenceBudgetStage 为 no-op。
    pub session_dir: Option<PathBuf>,
    /// 聚合预算配置。None 时 PersistenceBudgetStage 为 no-op。
    pub persistence_budget_config: Option<&'a PersistenceBudgetConfig>,
    /// 微压缩配置。None 时 MicroCompactStage 为 no-op。
    pub micro_compact_config: Option<&'a MicroCompactConfig>,
    /// 最后 assistant 输出的时间戳，供微压缩判断空闲时间。
    pub last_assistant_at: Option<chrono::DateTime<chrono::Utc>>,
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
/// 约束：stage 以同步纯变换为主，不做 IO、不发事件、不做审批，也不触发 compact。
/// 唯一例外：`PersistenceBudgetStage` 被正式授权做受控文件 IO（同步、幂等、失败降级），
/// 但只修改 `ContextBundle`，不修改 `AgentState` 或事件日志。
pub(crate) trait ContextStage: Send + Sync {
    fn apply(&self, bundle: ContextBundle, ctx: &ContextStageContext<'_>) -> Result<ContextBundle>;
}

/// Runtime-facing pipeline wrapper.
pub(crate) struct ContextRuntime {
    stages: Vec<Box<dyn ContextStage>>,
    tool_result_max_bytes: usize,
    persistence_budget_config: Option<PersistenceBudgetConfig>,
    micro_compact_config: Option<MicroCompactConfig>,
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
                Box::new(MicroCompactStage),
                Box::new(PersistenceBudgetStage),
                Box::new(PrunePassStage),
                Box::new(BudgetTrimStage),
            ],
            tool_result_max_bytes: tool_result_max_bytes.max(1),
            persistence_budget_config: None,
            micro_compact_config: None,
        }
    }

    pub fn with_persistence_budget_config(mut self, config: PersistenceBudgetConfig) -> Self {
        self.persistence_budget_config = Some(config);
        self
    }

    pub fn with_micro_compact_config(mut self, config: MicroCompactConfig) -> Self {
        self.micro_compact_config = Some(config);
        self
    }

    #[cfg(test)]
    fn from_stages(stages: Vec<Box<dyn ContextStage>>) -> Self {
        Self {
            stages,
            tool_result_max_bytes: 100_000, // 测试默认值，与 runtime-config 保持一致
            persistence_budget_config: None,
            micro_compact_config: None,
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
        let session_dir = astrcode_core::project::project_dir(&state.working_dir)
            .map(|dir| dir.join("sessions").join(&state.session_id))
            .ok();
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
            session_dir,
            persistence_budget_config: self.persistence_budget_config.as_ref(),
            micro_compact_config: self.micro_compact_config.as_ref(),
            last_assistant_at: state.last_assistant_at,
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

/// 时间触发微压缩：当会话空闲时间超过阈值时，
/// 清除标记为 `compact_clearable` 的旧工具结果，释放上下文空间。
///
/// 与 PrunePass 的区别：MicroCompact 基于时间触发（会话输出静默时间），
/// 而 PrunePass 基于 token 压力在每个 step 执行。
struct MicroCompactStage;

impl ContextStage for MicroCompactStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        if let Some(config) = ctx.micro_compact_config {
            if should_trigger(
                ctx.last_assistant_at,
                chrono::Utc::now(),
                config.gap_threshold_secs,
            ) {
                let stats = apply_micro_compact(
                    &mut bundle.conversation.messages,
                    ctx.capability_descriptors,
                    config.keep_recent_results,
                );
                bundle.micro_compact_stats = stats;
            }
        }
        Ok(bundle)
    }
}

/// 聚合预算持久化：当消息流中未持久化工具结果的总大小超过预算时，
/// 将最大的结果强制落盘并替换为 `<persisted-output>` 引用。
///
/// 这是管线中唯一被授权做受控文件 IO 的 stage：
/// - IO 操作是同步、幂等的（同一 tool_call_id 写一次，重复调用跳过）
/// - 不修改 AgentState 或事件日志（只修改 ContextBundle）
/// - 失败时降级（磁盘写入失败 → 不替换，让 PrunePass 截断兜底）
struct PersistenceBudgetStage;

impl ContextStage for PersistenceBudgetStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        if let Some(config) = ctx.persistence_budget_config {
            let stats = enforce_aggregate_budget(
                &mut bundle.conversation.messages,
                ctx.capability_descriptors,
                ctx.session_dir.as_deref(),
                config,
            );
            bundle.persistence_stats = stats;
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
}
