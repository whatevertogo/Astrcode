//! # Hook Runtime
//!
//! 这里不做“全局事件总线”。
//! 它只负责在明确的生命周期点按顺序执行 hook，并把可改变控制流的能力
//! 限制在少数前置节点，避免广播语义和拦截语义混在一起。

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

pub(crate) enum PreCompactDecision {
    Continue,
    Blocked { reason: String },
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

    pub(crate) async fn run_pre_compact(
        &self,
        compaction: CompactionHookContext,
    ) -> Result<PreCompactDecision> {
        let current = compaction;

        for handler in self.handlers_for_event(HookEvent::PreCompact) {
            let input = HookInput::PreCompact(current.clone());
            if !handler.matches(&input) {
                continue;
            }
            match handler.run(&input).await? {
                HookOutcome::Continue => {},
                HookOutcome::Block { reason } => {
                    return Ok(PreCompactDecision::Blocked {
                        reason: format!("hook '{}' blocked compaction: {reason}", handler.name()),
                    });
                },
                HookOutcome::ReplaceToolArgs { .. } => {
                    return Err(AstrError::Validation(format!(
                        "hook '{}' returned ReplaceToolArgs for PreCompact, which is not supported",
                        handler.name()
                    )));
                },
            }
        }

        Ok(PreCompactDecision::Continue)
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
