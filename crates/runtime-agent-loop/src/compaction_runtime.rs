//! Compaction runtime primitives.
//!
//! This module separates "should we compact", "how do we compact", and "what conversation view do
//! we rebuild" so the loop can swap strategies without inlining every branch into `turn_runner`.

use std::sync::{Arc, Mutex as StdMutex};

use astrcode_core::{
    format_compact_summary, project, CancelToken, CompactTrigger, ContextDecisionInput,
    ContextStrategy, LlmMessage, Result, StorageEvent, StoredEvent, UserMessageOrigin,
};
use astrcode_runtime_llm::LlmProvider;
use async_trait::async_trait;
use chrono::Utc;

use crate::context_pipeline::ConversationView;
use crate::context_window::{auto_compact, should_compact, CompactConfig, PromptTokenSnapshot};

/// Why a compaction attempt happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionReason {
    /// Triggered by token pressure before the LLM call.
    Auto,
    /// Triggered by a 413 prompt-too-long error during the LLM call.
    Reactive,
    /// Triggered explicitly by a user-initiated manual compact action.
    Manual,
}

impl CompactionReason {
    pub(crate) fn as_trigger(self) -> CompactTrigger {
        match self {
            Self::Auto | Self::Reactive => CompactTrigger::Auto,
            Self::Manual => CompactTrigger::Manual,
        }
    }

    pub(crate) fn as_context_strategy(self) -> ContextStrategy {
        match self {
            Self::Auto | Self::Reactive | Self::Manual => ContextStrategy::Compact,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventRange {
    pub start: usize,
    pub end: usize,
}

/// Internal artifact describing a completed compaction step.
#[derive(Debug, Clone)]
pub(crate) struct CompactionArtifact {
    pub summary: String,
    pub source_range: EventRange,
    pub preserved_tail_start: u64,
    pub strategy_id: String,
    pub pre_tokens: usize,
    pub post_tokens_estimate: usize,
    pub compacted_at_seq: u64,
    pub trigger: CompactionReason,
    pub preserved_recent_turns: usize,
    pub messages_removed: usize,
    pub tokens_freed: usize,
}

/// Real tail snapshot used when rebuilding a compacted conversation view.
///
/// `seed` contains the already-persisted recent tail before the active step starts. `live` can be
/// wired to the current turn's append path so reactive compact sees the exact events that were
/// persisted during this turn before the rebuild happens.
#[derive(Clone, Default)]
pub struct CompactionTailSnapshot {
    seed: Vec<StoredEvent>,
    live: Option<Arc<StdMutex<Vec<StoredEvent>>>>,
}

impl CompactionTailSnapshot {
    pub fn from_seed(seed: Vec<StoredEvent>) -> Self {
        Self { seed, live: None }
    }

    pub fn from_messages(messages: &[LlmMessage], keep_recent_turns: usize) -> Self {
        Self::from_seed(tail_snapshot_from_messages(messages, keep_recent_turns))
    }

    pub fn with_live_recorder(mut self, live: Arc<StdMutex<Vec<StoredEvent>>>) -> Self {
        self.live = Some(live);
        self
    }

    pub fn materialize(&self) -> Vec<StoredEvent> {
        let mut tail = self.seed.clone();
        if let Some(live) = &self.live {
            tail.extend(live.lock().expect("compaction tail lock").iter().cloned());
        }
        tail
    }
}

fn tail_snapshot_from_messages(
    messages: &[LlmMessage],
    preserved_recent_turns: usize,
) -> Vec<StoredEvent> {
    let keep_start =
        recent_turn_start_index(messages, preserved_recent_turns).unwrap_or(messages.len());
    let timestamp = Utc::now();
    let mut current_turn = 0usize;

    messages[keep_start..]
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let turn_id = match message {
                LlmMessage::User {
                    origin: UserMessageOrigin::User,
                    ..
                } => {
                    current_turn += 1;
                    Some(format!("tail-turn-{current_turn}"))
                }
                _ if current_turn > 0 => Some(format!("tail-turn-{current_turn}")),
                _ => None,
            };

            let event = match message {
                LlmMessage::User { content, origin } => StorageEvent::UserMessage {
                    turn_id,
                    content: content.clone(),
                    origin: *origin,
                    timestamp,
                },
                LlmMessage::Assistant {
                    content, reasoning, ..
                } => StorageEvent::AssistantFinal {
                    turn_id,
                    content: content.clone(),
                    reasoning_content: reasoning.as_ref().map(|value| value.content.clone()),
                    reasoning_signature: reasoning
                        .as_ref()
                        .and_then(|value| value.signature.clone()),
                    timestamp: Some(timestamp),
                },
                LlmMessage::Tool {
                    tool_call_id,
                    content,
                } => StorageEvent::ToolResult {
                    turn_id,
                    tool_call_id: tool_call_id.clone(),
                    tool_name: "tail.rebuild".to_string(),
                    success: true,
                    output: content.clone(),
                    error: None,
                    metadata: None,
                    duration_ms: 0,
                },
            };

            StoredEvent {
                storage_seq: (index + 1) as u64,
                event,
            }
        })
        .collect()
}

fn recent_turn_start_index(
    messages: &[LlmMessage],
    preserved_recent_turns: usize,
) -> Option<usize> {
    let mut seen_turns = 0usize;
    let mut last_index = None;

    for (index, message) in messages.iter().enumerate().rev() {
        if matches!(
            message,
            LlmMessage::User {
                origin: UserMessageOrigin::User,
                ..
            }
        ) {
            seen_turns += 1;
            last_index = Some(index);
            if seen_turns >= preserved_recent_turns {
                break;
            }
        }
    }

    last_index
}

pub(crate) struct CompactionInput<'a> {
    pub provider: &'a dyn LlmProvider,
    pub conversation: &'a ConversationView,
    pub base_system_prompt: Option<&'a str>,
    pub cancel: CancelToken,
    pub keep_recent_turns: usize,
    pub reason: CompactionReason,
}

pub(crate) trait CompactionPolicy: Send + Sync {
    fn should_compact(&self, snapshot: &PromptTokenSnapshot) -> Option<CompactionReason>;
}

#[async_trait]
pub(crate) trait CompactionStrategy: Send + Sync {
    async fn compact(&self, input: CompactionInput<'_>) -> Result<Option<CompactionArtifact>>;
}

pub(crate) trait CompactionRebuilder: Send + Sync {
    fn rebuild(
        &self,
        artifact: &CompactionArtifact,
        tail: &[StoredEvent],
    ) -> Result<ConversationView>;
}

pub(crate) struct CompactionRuntime {
    enabled: bool,
    keep_recent_turns: usize,
    pub(crate) policy: Arc<dyn CompactionPolicy>,
    pub(crate) strategy: Arc<dyn CompactionStrategy>,
    pub(crate) rebuilder: Arc<dyn CompactionRebuilder>,
}

impl CompactionRuntime {
    pub(crate) fn new(
        enabled: bool,
        keep_recent_turns: usize,
        policy: Arc<dyn CompactionPolicy>,
        strategy: Arc<dyn CompactionStrategy>,
        rebuilder: Arc<dyn CompactionRebuilder>,
    ) -> Self {
        Self {
            enabled,
            keep_recent_turns: keep_recent_turns.max(1),
            policy,
            strategy,
            rebuilder,
        }
    }

    pub(crate) fn auto_compact_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn keep_recent_turns(&self) -> usize {
        self.keep_recent_turns
    }

    pub(crate) fn build_context_decision(
        &self,
        snapshot: &PromptTokenSnapshot,
        truncated_tool_results: usize,
    ) -> ContextDecisionInput {
        // Always surface a decision input so the global PolicyEngine remains the final arbiter.
        let suggested_strategy = if self.enabled {
            self.policy
                .should_compact(snapshot)
                .map(CompactionReason::as_context_strategy)
                .unwrap_or(ContextStrategy::Ignore)
        } else {
            ContextStrategy::Ignore
        };

        ContextDecisionInput {
            estimated_tokens: snapshot.context_tokens,
            context_window: snapshot.context_window,
            effective_window: snapshot.effective_window,
            threshold_tokens: snapshot.threshold_tokens,
            truncated_tool_results,
            suggested_strategy,
        }
    }

    pub(crate) async fn compact(
        &self,
        provider: &dyn LlmProvider,
        conversation: &ConversationView,
        base_system_prompt: Option<&str>,
        reason: CompactionReason,
        cancel: CancelToken,
    ) -> Result<Option<CompactionArtifact>> {
        self.strategy
            .compact(CompactionInput {
                provider,
                conversation,
                base_system_prompt,
                cancel,
                keep_recent_turns: self.keep_recent_turns,
                reason,
            })
            .await
    }

    pub(crate) fn rebuild_conversation(
        &self,
        artifact: &CompactionArtifact,
        tail: &[StoredEvent],
    ) -> Result<ConversationView> {
        self.rebuilder.rebuild(artifact, tail)
    }
}

/// Default threshold-based policy that mirrors the existing `should_compact` helper.
///
/// This policy only provides a local hint. Even when it returns `None`, the loop still asks the
/// global `PolicyEngine` with `ContextStrategy::Ignore` as the suggested strategy so there is only
/// one final decision source for context handling.
pub(crate) struct ThresholdCompactionPolicy {
    enabled: bool,
}

impl ThresholdCompactionPolicy {
    pub(crate) fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}

impl CompactionPolicy for ThresholdCompactionPolicy {
    fn should_compact(&self, snapshot: &PromptTokenSnapshot) -> Option<CompactionReason> {
        if self.enabled && should_compact(*snapshot) {
            Some(CompactionReason::Auto)
        } else {
            None
        }
    }
}

/// Adapter over the existing `context_window::auto_compact` algorithm.
pub(crate) struct AutoCompactStrategy;

#[async_trait]
impl CompactionStrategy for AutoCompactStrategy {
    async fn compact(&self, input: CompactionInput<'_>) -> Result<Option<CompactionArtifact>> {
        let compact_result = auto_compact(
            input.provider,
            &input.conversation.messages,
            input.base_system_prompt,
            CompactConfig {
                keep_recent_turns: input.keep_recent_turns,
                trigger: input.reason.as_trigger(),
            },
            input.cancel,
        )
        .await?;

        Ok(compact_result.map(|result| CompactionArtifact {
            summary: result.summary,
            source_range: EventRange {
                start: 0,
                end: result.messages_removed,
            },
            preserved_tail_start: result.messages_removed as u64,
            strategy_id: "suffix_preserving_summary".to_string(),
            pre_tokens: result.pre_tokens,
            post_tokens_estimate: result.post_tokens_estimate,
            compacted_at_seq: 0, // TODO: wire real storage_seq from event log for session rebuild & debugging
            trigger: input.reason,
            preserved_recent_turns: result.preserved_recent_turns,
            messages_removed: result.messages_removed,
            tokens_freed: result.tokens_freed,
        }))
    }
}

/// Default rebuilder that projects the preserved real tail and prepends the compact summary.
pub(crate) struct ConversationViewRebuilder;

impl CompactionRebuilder for ConversationViewRebuilder {
    fn rebuild(
        &self,
        artifact: &CompactionArtifact,
        tail: &[StoredEvent],
    ) -> Result<ConversationView> {
        let projected_tail = project(
            &tail
                .iter()
                .map(|stored| stored.event.clone())
                .collect::<Vec<_>>(),
        );
        let mut messages = vec![astrcode_core::LlmMessage::User {
            content: format_compact_summary(&artifact.summary),
            origin: UserMessageOrigin::CompactSummary,
        }];
        messages.extend(projected_tail.messages);
        Ok(ConversationView::new(messages))
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmMessage, StorageEvent, UserMessageOrigin};

    use super::*;

    #[test]
    fn threshold_policy_returns_auto_only_when_snapshot_exceeds_threshold() {
        let policy = ThresholdCompactionPolicy::new(true);
        let snapshot = PromptTokenSnapshot {
            context_tokens: 91,
            budget_tokens: 91,
            context_window: 100,
            effective_window: 90,
            threshold_tokens: 90,
        };

        assert_eq!(
            policy.should_compact(&snapshot),
            Some(CompactionReason::Auto)
        );
    }

    #[test]
    fn rebuilder_returns_conversation_view_from_artifact() {
        let artifact = CompactionArtifact {
            summary: "summary".to_string(),
            source_range: EventRange { start: 0, end: 1 },
            preserved_tail_start: 1,
            strategy_id: "test".to_string(),
            pre_tokens: 100,
            post_tokens_estimate: 40,
            compacted_at_seq: 0,
            trigger: CompactionReason::Auto,
            preserved_recent_turns: 1,
            messages_removed: 1,
            tokens_freed: 60,
        };
        let tail = vec![StoredEvent {
            storage_seq: 1,
            event: StorageEvent::UserMessage {
                turn_id: Some("turn-1".to_string()),
                content: "current ask".to_string(),
                origin: UserMessageOrigin::User,
                timestamp: Utc::now(),
            },
        }];

        let rebuilt = ConversationViewRebuilder
            .rebuild(&artifact, &tail)
            .expect("rebuild should succeed");

        assert_eq!(rebuilt.messages.len(), 2);
        assert!(matches!(
            &rebuilt.messages[0],
            LlmMessage::User { content, .. } if content.contains("summary")
        ));
        assert!(matches!(
            &rebuilt.messages[1],
            LlmMessage::User { content, .. } if content == "current ask"
        ));
    }

    #[test]
    fn build_context_decision_keeps_global_policy_in_the_loop_when_local_policy_skips_compact() {
        let runtime = CompactionRuntime::new(
            true,
            1,
            Arc::new(ThresholdCompactionPolicy::new(true)),
            Arc::new(AutoCompactStrategy),
            Arc::new(ConversationViewRebuilder),
        );
        let snapshot = PromptTokenSnapshot {
            context_tokens: 10,
            budget_tokens: 10,
            context_window: 100,
            effective_window: 90,
            threshold_tokens: 90,
        };

        let decision = runtime.build_context_decision(&snapshot, 2);

        assert_eq!(decision.suggested_strategy, ContextStrategy::Ignore);
        assert_eq!(decision.truncated_tool_results, 2);
    }

    #[test]
    fn build_context_decision_uses_ignore_when_auto_compact_is_disabled() {
        let runtime = CompactionRuntime::new(
            false,
            1,
            Arc::new(ThresholdCompactionPolicy::new(false)),
            Arc::new(AutoCompactStrategy),
            Arc::new(ConversationViewRebuilder),
        );
        let snapshot = PromptTokenSnapshot {
            context_tokens: 95,
            budget_tokens: 95,
            context_window: 100,
            effective_window: 90,
            threshold_tokens: 90,
        };

        let decision = runtime.build_context_decision(&snapshot, 0);

        assert_eq!(decision.suggested_strategy, ContextStrategy::Ignore);
    }
}
