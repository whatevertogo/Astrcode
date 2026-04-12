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
    PersistenceStats, PruneStats,
    micro_compact::{MicroCompactConfig, MicroCompactStats},
    tool_result_persistence::PersistenceBudgetConfig,
};

mod stages;
use stages::default_stages;

#[cfg(test)]
mod tests;

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
            stages: default_stages(),
            tool_result_max_bytes: tool_result_max_bytes.max(1),
            persistence_budget_config: None,
            micro_compact_config: None,
        }
    }

    pub fn with_persistence_budget_config(mut self, config: PersistenceBudgetConfig) -> Self {
        self.persistence_budget_config = Some(config);
        self
    }

    pub fn with_tool_result_max_bytes(mut self, tool_result_max_bytes: usize) -> Self {
        self.tool_result_max_bytes = tool_result_max_bytes.max(1);
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
