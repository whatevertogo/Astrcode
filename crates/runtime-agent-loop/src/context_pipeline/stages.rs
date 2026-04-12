use astrcode_core::Result;

use super::{
    ContextBlock, ContextBundle, ContextDiagnostic, ContextStage, ContextStageContext,
    ConversationView,
};
use crate::context_window::{
    apply_prune_pass, effective_context_window,
    micro_compact::{apply_micro_compact, should_trigger},
    tool_result_persistence::enforce_aggregate_budget,
};

/// 构造默认的 context pipeline stages。
pub(super) fn default_stages() -> Vec<Box<dyn ContextStage>> {
    vec![
        Box::new(BaselineStage),
        Box::new(RecentTailStage),
        Box::new(WorksetStage),
        Box::new(CompactionViewStage),
        Box::new(RecoveryContextStage),
        Box::new(MicroCompactStage),
        Box::new(PersistenceBudgetStage),
        Box::new(PrunePassStage),
        Box::new(BudgetTrimStage),
    ]
}

/// 将 AgentState 中的投影消息物化为初始对话视图。
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

/// 预留给未来的 tail-focused materialization。
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

/// 注入最小 workset 槽位，给后续阶段稳定的结构化扩展点。
struct WorksetStage;

impl ContextStage for WorksetStage {
    fn apply(
        &self,
        mut bundle: ContextBundle,
        ctx: &ContextStageContext<'_>,
    ) -> Result<ContextBundle> {
        // 保持 workset 槽位从第一天起就存在，后续增强时不需要改 bundle 形状。
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

/// 如果 compaction 已经重建了更窄的会话视图，用它覆盖 baseline 视图。
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
/// 恢复块进入 memory 槽位，而不是重新混入 conversation messages。
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

/// 在模型可见的 conversation 上执行本地 prune。
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

/// 预留给未来的 token-budget-aware trimming。
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
