//! # Hook Runtime
//!
//! 这里不做"全局事件总线"。
//! 它只负责在明确的生命周期点按顺序执行 hook，并把可改变控制流的能力
//! 限制在少数前置节点，避免广播语义和拦截语义混在一起。
//!
//! ## PreCompact Hook 修改能力
//!
//! `run_pre_compact` 支持三种返回：
//! - `Continue`: 正常执行压缩
//! - `Blocked`: 阻止压缩
//! - `Modified`: 携带修改参数（system_prompt / keep_recent_turns / custom_summary）
//!
//! 多个 hook 的修改会链式合并，后执行的 hook 可以覆盖前面 hook 的修改。

use std::sync::Arc;

use astrcode_core::{
    AstrError, CompactionHookContext, CompactionHookResultContext, HookEvent, HookHandler,
    HookInput, HookOutcome, Result, ToolHookContext, ToolHookResultContext,
};

#[derive(Default)]
pub(crate) struct HookRuntime {
    handlers: Vec<Arc<dyn HookHandler>>,
}

pub(crate) enum PreToolUseDecision {
    Continue(ToolHookContext),
    Blocked {
        reason: String,
        tool: ToolHookContext,
    },
}

/// PreCompact hook 的决策结果。
///
/// 支持三种控制方式：
/// - `Continue`: 允许压缩继续，不做任何修改
/// - `Blocked`: 阻止本次压缩
/// - `Modified`: 携带修改参数，允许 hook 修改压缩行为
#[derive(Clone, Debug, Default)]
pub(crate) struct PreCompactDecision {
    /// 是否允许压缩继续。
    pub allowed: bool,
    /// 阻止原因（如果 `allowed` 为 false）。
    pub block_reason: Option<String>,
    /// 覆盖的 system prompt（如果提供）。
    pub override_system_prompt: Option<String>,
    /// 覆盖的保留最近 turn 数量（如果提供）。
    pub override_keep_recent_turns: Option<usize>,
    /// 自定义摘要内容（如果提供，跳过 LLM 调用）。
    pub custom_summary: Option<String>,
}

impl PreCompactDecision {
    /// 创建一个允许继续的决策（不做修改）。
    pub(crate) fn continue_() -> Self {
        Self {
            allowed: true,
            ..Default::default()
        }
    }

    /// 创建一个阻止压缩的决策。
    pub(crate) fn blocked(reason: String) -> Self {
        Self {
            allowed: false,
            block_reason: Some(reason),
            ..Default::default()
        }
    }

    /// 合并另一个决策的修改到当前决策。
    /// 后执行的 hook 可以覆盖前面 hook 的修改。
    pub(crate) fn merge(&mut self, other: PreCompactModification) {
        if let Some(prompt) = other.override_system_prompt {
            self.override_system_prompt = Some(prompt);
        }
        if let Some(turns) = other.override_keep_recent_turns {
            self.override_keep_recent_turns = Some(turns);
        }
        if let Some(summary) = other.custom_summary {
            self.custom_summary = Some(summary);
        }
    }
}

/// 从 hook 返回的压缩修改参数。
#[derive(Clone, Debug, Default)]
pub(crate) struct PreCompactModification {
    pub override_system_prompt: Option<String>,
    pub override_keep_recent_turns: Option<usize>,
    pub custom_summary: Option<String>,
}

impl HookRuntime {
    pub(crate) fn register(&mut self, handler: Arc<dyn HookHandler>) {
        self.handlers.push(handler);
    }

    pub(crate) fn register_all<I>(&mut self, handlers: I)
    where
        I: IntoIterator<Item = Arc<dyn HookHandler>>,
    {
        self.handlers.extend(handlers);
    }

    pub(crate) async fn run_pre_tool_use(
        &self,
        tool: ToolHookContext,
    ) -> Result<PreToolUseDecision> {
        let mut current = tool;

        for handler in self.handlers_for_event(HookEvent::PreToolUse) {
            let input = HookInput::PreToolUse(current.clone());
            if !handler.matches(&input) {
                continue;
            }
            match handler.run(&input).await? {
                HookOutcome::Continue => {},
                HookOutcome::Block { reason } => {
                    return Ok(PreToolUseDecision::Blocked {
                        reason: format!("hook '{}' blocked tool call: {reason}", handler.name()),
                        tool: current,
                    });
                },
                HookOutcome::ReplaceToolArgs { args } => {
                    current.args = args;
                },
                HookOutcome::ModifyCompactContext { .. } => {
                    return Err(AstrError::Validation(format!(
                        "hook '{}' returned ModifyCompactContext for PreToolUse, which is not \
                         supported",
                        handler.name()
                    )));
                },
            }
        }

        Ok(PreToolUseDecision::Continue(current))
    }

    pub(crate) async fn run_post_tool_use_best_effort(&self, input: ToolHookResultContext) {
        self.run_post_hook_best_effort(HookInput::PostToolUse(input))
            .await;
    }

    pub(crate) async fn run_post_tool_failure_best_effort(&self, input: ToolHookResultContext) {
        self.run_post_hook_best_effort(HookInput::PostToolUseFailure(input))
            .await;
    }

    /// 执行 PreCompact hooks，支持修改压缩参数。
    ///
    /// 多个 hook 的修改会链式合并：
    /// 1. 每个 hook 可以选择 `Continue`、`Block` 或 `ModifyCompactContext`
    /// 2. `ModifyCompactContext` 的修改会累积到 `PreCompactDecision`
    /// 3. 后执行的 hook 可以覆盖前面 hook 的修改
    /// 4. 任何一个 hook 返回 `Block` 会立即终止并阻止压缩
    pub(crate) async fn run_pre_compact(
        &self,
        compaction: CompactionHookContext,
    ) -> Result<PreCompactDecision> {
        let current = compaction;
        let mut decision = PreCompactDecision::continue_();

        for handler in self.handlers_for_event(HookEvent::PreCompact) {
            let input = HookInput::PreCompact(current.clone());
            if !handler.matches(&input) {
                continue;
            }
            match handler.run(&input).await? {
                HookOutcome::Continue => {},
                HookOutcome::Block { reason } => {
                    return Ok(PreCompactDecision::blocked(format!(
                        "hook '{}' blocked compaction: {reason}",
                        handler.name()
                    )));
                },
                HookOutcome::ReplaceToolArgs { .. } => {
                    return Err(AstrError::Validation(format!(
                        "hook '{}' returned ReplaceToolArgs for PreCompact, which is not supported",
                        handler.name()
                    )));
                },
                HookOutcome::ModifyCompactContext {
                    override_system_prompt,
                    override_keep_recent_turns,
                    custom_summary,
                } => {
                    log::debug!(
                        "hook '{}' modified compact context: prompt={}, turns={}, summary={}",
                        handler.name(),
                        override_system_prompt.is_some(),
                        override_keep_recent_turns.is_some(),
                        custom_summary.is_some()
                    );
                    decision.merge(PreCompactModification {
                        override_system_prompt,
                        override_keep_recent_turns,
                        custom_summary,
                    });
                },
            }
        }

        Ok(decision)
    }

    pub(crate) async fn run_post_compact_best_effort(&self, input: CompactionHookResultContext) {
        self.run_post_hook_best_effort(HookInput::PostCompact(input))
            .await;
    }

    async fn run_post_hook_best_effort(&self, input: HookInput) {
        let event = input.event();
        for handler in self.handlers_for_event(event) {
            if !handler.matches(&input) {
                continue;
            }
            match handler.run(&input).await {
                Ok(HookOutcome::Continue) => {},
                Ok(HookOutcome::Block { reason }) => {
                    log::warn!(
                        "hook '{}' returned Block for {:?}, ignoring because post hooks are \
                         best-effort: {}",
                        handler.name(),
                        event,
                        reason
                    );
                },
                Ok(HookOutcome::ReplaceToolArgs { .. }) => {
                    log::warn!(
                        "hook '{}' returned ReplaceToolArgs for {:?}, ignoring because post hooks \
                         are best-effort",
                        handler.name(),
                        event
                    );
                },
                Ok(HookOutcome::ModifyCompactContext { .. }) => {
                    log::warn!(
                        "hook '{}' returned ModifyCompactContext for {:?}, ignoring because post \
                         hooks are best-effort",
                        handler.name(),
                        event
                    );
                },
                Err(error) => {
                    log::error!(
                        "hook '{}' failed during {:?}: {}",
                        handler.name(),
                        event,
                        error
                    );
                },
            }
        }
    }

    fn handlers_for_event(&self, event: HookEvent) -> impl Iterator<Item = &Arc<dyn HookHandler>> {
        self.handlers
            .iter()
            .filter(move |handler| handler.event() == event)
    }
}
