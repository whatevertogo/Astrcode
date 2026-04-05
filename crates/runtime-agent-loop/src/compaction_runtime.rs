//! # Compaction Runtime（压缩运行时）
//!
//! ## 职责
//!
//! 统一管理上下文压缩的触发策略、决策和执行。
//! 将"是否应该压缩"、"如何压缩"和"重建什么会话视图"三个关注点分离，
//! 使 AgentLoop 可以在不将每个分支内联到 `turn_runner` 的情况下切换策略。
//!
//! ## 在 Turn 流程中的作用
//!
//! - **调用时机 1**：每个 step 中，`policy.decide_context_strategy()` 评估是否需要压缩
//! - **调用时机 2**：若策略返回 `Compact`，`turn_runner` 调用 `compact()` 执行压缩
//! - **调用时机 3**：LLM 返回 413 prompt-too-long 时，触发 reactive compact
//! - **调用时机 4**：用户手动触发压缩时，`manual_compact_event()` 被调用
//! - **输入**：LLM Provider、当前 `ConversationView`、可选的 system prompt、压缩原因
//! - **输出**：可选的 `CompactionArtifact` + 重建后的 `ConversationView`
//!
//! ## 依赖和协作
//!
//! - **使用** `astrcode_core::{CompactTrigger, ContextStrategy, ContextDecisionInput}`
//!   进行策略决策和触发评估
//! - **使用** `auto_compact()` / `should_compact()` 执行实际的 token 级压缩
//! - **使用** `LlmMessage` 序列调用 LLM 生成压缩摘要
//! - **被调用方**：`turn_runner` 中的 `maybe_compact_conversation()` 辅助函数
//! - **被调用方**：`AgentLoop::manual_compact_event()` 用于用户主动压缩
//! - **输出给**：`turn_runner` 将 `compacted_view` 赋值给本地 `conversation` 变量， 并在下一个 step
//!   中通过 `prior_compaction_view` 传给 `ContextPipeline`
//!
//! ## 三种压缩原因
//!
//! | 原因 | 触发时机 | 策略 |
//! |------|----------|------|
//! | `Auto` | 上下文窗口使用率超过阈值 | `ThresholdCompactionPolicy` |
//! | `Reactive` | LLM 返回 413 prompt-too-long | 自动触发，最多重试 3 次 |
//! | `Manual` | 用户主动触发 | 与 Auto 共享同一条压缩路径 |
//!
//! ## 关键设计
//!
//! - `CompactionRuntime` 持有三个协作者：`policy`（触发策略）、`strategy`（自动压缩策略）、
//!   `rebuilder`（对话视图重建器），各自通过 trait 抽象，可独立替换
//! - `CompactionTailSnapshot` 携带最近 N 个 turn 的消息快照，用于压缩后保留尾部上下文
//! - `rebuild_conversation()` 将 artifact 转换为 `ConversationView`，供 pipeline 注入

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use astrcode_core::{
    CancelToken, CompactTrigger, ContextDecisionInput, ContextStrategy, LlmMessage, Result,
    StorageEvent, StoredEvent, UserMessageOrigin, format_compact_summary, project,
};
use astrcode_runtime_llm::LlmProvider;
use async_trait::async_trait;
use chrono::Utc;

use crate::{
    context_pipeline::ConversationView,
    context_window::{CompactConfig, PromptTokenSnapshot, auto_compact, should_compact},
};

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
    /// 压缩后需要恢复的文件路径列表（由 FileAccessTracker 跟踪）。
    pub recovered_files: Vec<PathBuf>,
}

impl CompactionArtifact {
    /// Stamp the artifact with the newest durable tail sequence that survived the compaction.
    ///
    /// The strategy itself only sees model-visible messages, so it cannot know which real
    /// `storage_seq` the preserved tail corresponds to. We fill this in after the loop/materializer
    /// has reconstructed the actual stored tail snapshot, which keeps rebuild/debug metadata tied
    /// to session truth instead of synthetic message indexes.
    pub(crate) fn record_tail_seq(&mut self, tail: &[StoredEvent]) {
        if let Some(max_seq) = tail.iter().map(|stored| stored.storage_seq).max() {
            self.compacted_at_seq = max_seq;
        }
    }
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
                },
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
    /// 仅作为压缩模板的“上下文参考”嵌入，不是最终发送给 compact LLM 的完整模板。
    pub compact_prompt_context: Option<&'a str>,
    pub cancel: CancelToken,
    pub keep_recent_turns: usize,
    pub reason: CompactionReason,
}

pub(crate) trait CompactionPolicy: Send + Sync {
    fn should_compact(&self, snapshot: &PromptTokenSnapshot) -> Option<CompactionReason>;

    /// Called after a successful compaction. Resets the circuit breaker if applicable.
    fn record_success(&self) {}

    /// Called after a failed compaction. Increments the circuit breaker if applicable.
    fn record_failure(&self) {}
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
        file_contents: &[(PathBuf, String)],
    ) -> Result<ConversationView>;
}

/// 文件内容读取抽象，用于 post-compact 文件恢复。
///
/// 将文件读取与 `std::fs` 解耦，便于测试时注入 mock。
pub(crate) trait FileContentProvider: Send + Sync {
    fn read_to_string(&self, path: &std::path::Path) -> std::io::Result<String>;
}

/// 生产环境使用 `std::fs::read_to_string` 读取文件。
pub(crate) struct FsFileContentProvider;

impl FileContentProvider for FsFileContentProvider {
    fn read_to_string(&self, path: &std::path::Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }
}

pub(crate) struct CompactionRuntime {
    enabled: bool,
    keep_recent_turns: usize,
    threshold_percent: u8,
    pub(crate) policy: Arc<dyn CompactionPolicy>,
    pub(crate) strategy: Arc<dyn CompactionStrategy>,
    pub(crate) rebuilder: Arc<dyn CompactionRebuilder>,
    pub(crate) file_provider: Arc<dyn FileContentProvider>,
}

/// Post-compact 文件恢复的最大文件数。
pub(crate) const MAX_RECOVERED_FILES: usize = 5;
/// 文件恢复的总 token 预算（估算值）。
const RECOVERY_TOKEN_BUDGET: usize = 50_000;

impl CompactionRuntime {
    pub(crate) fn new(
        enabled: bool,
        keep_recent_turns: usize,
        threshold_percent: u8,
        policy: Arc<dyn CompactionPolicy>,
        strategy: Arc<dyn CompactionStrategy>,
        rebuilder: Arc<dyn CompactionRebuilder>,
        file_provider: Arc<dyn FileContentProvider>,
    ) -> Self {
        Self {
            enabled,
            keep_recent_turns: keep_recent_turns.max(1),
            threshold_percent: threshold_percent.clamp(1, 100),
            policy,
            strategy,
            rebuilder,
            file_provider,
        }
    }

    pub(crate) fn auto_compact_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn keep_recent_turns(&self) -> usize {
        self.keep_recent_turns
    }

    pub(crate) fn threshold_percent(&self) -> u8 {
        self.threshold_percent
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

    /// 使用配置中的默认保留轮数执行压缩
    ///
    /// 仅用于测试。生产代码使用 `compact_with_keep_recent_turns` 以便 hook 修改保留轮数。
    #[cfg(test)]
    pub(crate) async fn compact(
        &self,
        provider: &dyn LlmProvider,
        conversation: &ConversationView,
        compact_prompt_context: Option<&str>,
        reason: CompactionReason,
        cancel: CancelToken,
    ) -> Result<Option<CompactionArtifact>> {
        self.compact_with_keep_recent_turns(
            provider,
            conversation,
            compact_prompt_context,
            self.keep_recent_turns,
            reason,
            cancel,
        )
        .await
    }

    /// 使用显式保留轮数执行压缩。
    ///
    /// 当 hook 修改了 `keep_recent_turns` 时使用此方法。
    /// 与 `compact` 不同，此方法不会自动使用配置中的 `keep_recent_turns`。
    pub(crate) async fn compact_with_keep_recent_turns(
        &self,
        provider: &dyn LlmProvider,
        conversation: &ConversationView,
        compact_prompt_context: Option<&str>,
        keep_recent_turns: usize,
        reason: CompactionReason,
        cancel: CancelToken,
    ) -> Result<Option<CompactionArtifact>> {
        let result = self
            .strategy
            .compact(CompactionInput {
                provider,
                conversation,
                compact_prompt_context,
                cancel,
                keep_recent_turns: keep_recent_turns.max(1),
                reason,
            })
            .await;

        match &result {
            Ok(Some(_)) => self.policy.record_success(),
            Ok(None) => {},
            Err(_) => {
                // Only record failures for automatic triggers; manual compaction
                // should not affect the circuit breaker.
                if !matches!(reason, CompactionReason::Manual) {
                    self.policy.record_failure();
                }
            },
        }

        result
    }

    /// 手动压缩使用显式保留轮数。
    ///
    /// 手动 `/compact` 的语义是“现在就尽量压缩”，不应因为自动压缩配置里的
    /// `keep_recent_turns` 过大而直接退化成 no-op。
    pub(crate) async fn compact_manual_with_keep_recent_turns(
        &self,
        provider: &dyn LlmProvider,
        conversation: &ConversationView,
        compact_prompt_context: Option<&str>,
        keep_recent_turns: usize,
        cancel: CancelToken,
    ) -> Result<Option<CompactionArtifact>> {
        let result = self
            .strategy
            .compact(CompactionInput {
                provider,
                conversation,
                compact_prompt_context,
                cancel,
                keep_recent_turns: keep_recent_turns.max(1),
                reason: CompactionReason::Manual,
            })
            .await;

        if result.as_ref().is_err() {
            // 手动 compact 不参与自动压缩熔断器统计，因此这里不记录失败次数。
        }

        result
    }

    pub(crate) fn rebuild_conversation(
        &self,
        artifact: &CompactionArtifact,
        tail: &[StoredEvent],
    ) -> Result<ConversationView> {
        // 从 FileAccessTracker 跟踪的文件路径中恢复文件内容
        let file_contents = self.recover_file_contents(&artifact.recovered_files);
        self.rebuilder.rebuild(artifact, tail, &file_contents)
    }

    /// 读取恢复文件的最近内容，受 MAX_RECOVERED_FILES 和 token 预算限制。
    ///
    /// 调用方负责在 compact 完成后填充 `artifact.recovered_files`。
    /// 这样策略层仍保持只关心“如何压缩”，而 turn/服务层负责把最近的文件访问
    /// 事实接回重建阶段，避免压缩后丢失刚读过的重要代码上下文。
    fn recover_file_contents(&self, paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
        let mut results = Vec::new();
        let mut used_tokens = 0usize;
        for path in paths.iter().take(MAX_RECOVERED_FILES) {
            match self.file_provider.read_to_string(path) {
                Ok(content) => {
                    let tokens = crate::context_window::estimate_text_tokens(&content);
                    if used_tokens + tokens > RECOVERY_TOKEN_BUDGET {
                        log::warn!(
                            "post-compact file recovery: token budget exhausted at {} tokens, \
                             skipping remaining files",
                            used_tokens
                        );
                        break;
                    }
                    used_tokens += tokens;
                    results.push((path.clone(), content));
                },
                Err(e) => {
                    log::warn!(
                        "post-compact file recovery: failed to read {}: {e}",
                        path.display()
                    );
                },
            }
        }
        results
    }
}

/// Default threshold-based policy that mirrors the existing `should_compact` helper.
///
/// This policy only provides a local hint. Even when it returns `None`, the loop still asks the
/// global `PolicyEngine` with `ContextStrategy::Ignore` as the suggested strategy so there is only
/// one final decision source for context handling.
///
/// Includes a circuit breaker: after `MAX_CONSECUTIVE_FAILURES` consecutive failures,
/// `should_compact()` returns `None` to prevent wasting API calls. Manual compaction bypasses
/// this check entirely.
pub(crate) const MAX_CONSECUTIVE_FAILURES: usize = 3;

pub(crate) struct ThresholdCompactionPolicy {
    enabled: bool,
    consecutive_failures: AtomicUsize,
}

impl ThresholdCompactionPolicy {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            consecutive_failures: AtomicUsize::new(0),
        }
    }
}

impl CompactionPolicy for ThresholdCompactionPolicy {
    fn should_compact(&self, snapshot: &PromptTokenSnapshot) -> Option<CompactionReason> {
        if !self.enabled || !should_compact(*snapshot) {
            return None;
        }
        if self.consecutive_failures.load(Ordering::Relaxed) >= MAX_CONSECUTIVE_FAILURES {
            log::warn!(
                "circuit breaker open: skipping auto-compaction after {} consecutive failures \
                 (consecutive_failures={})",
                MAX_CONSECUTIVE_FAILURES,
                self.consecutive_failures.load(Ordering::Relaxed),
            );
            return None;
        }
        Some(CompactionReason::Auto)
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    fn record_failure(&self) {
        let prev = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        log::warn!("auto-compaction failed (consecutive_failures={})", prev + 1);
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
            input.compact_prompt_context,
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
            // Filled by the loop after it materializes the real stored tail snapshot.
            compacted_at_seq: 0,
            trigger: input.reason,
            preserved_recent_turns: result.preserved_recent_turns,
            messages_removed: result.messages_removed,
            tokens_freed: result.tokens_freed,
            // 文件恢复路径由调用方在压缩后注入，策略层不持有 FileAccessTracker
            recovered_files: Vec::new(),
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
        file_contents: &[(PathBuf, String)],
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

        // 注入恢复的文件内容，作为压缩摘要和尾部消息之间的上下文补充
        for (path, content) in file_contents {
            // 截断过长的文件内容，保留前 30K 字节
            // 使用 char 边界安全截断，避免在多字节 UTF-8 字符中间切割导致 panic
            let mut truncate_at = content.len().min(30_000);
            while !content.is_char_boundary(truncate_at) && truncate_at > 0 {
                truncate_at -= 1;
            }
            let truncated = &content[..truncate_at];
            messages.push(LlmMessage::User {
                content: format!(
                    "[Post-compact file recovery: {}]\n```\n{}\n```",
                    path.display(),
                    truncated
                ),
                origin: UserMessageOrigin::CompactSummary,
            });
        }

        messages.extend(projected_tail.messages);
        Ok(ConversationView::new(messages))
    }
}

/// 从自定义摘要构建 CompactionArtifact。
///
/// 当 hook 提供 custom_summary 时使用此函数，跳过 LLM 压缩调用。
/// 这允许插件完全接管压缩逻辑（例如使用外部服务生成摘要）。
pub(crate) fn build_artifact_from_custom_summary(
    messages: &[astrcode_core::LlmMessage],
    custom_summary: &str,
    keep_recent_turns: usize,
    reason: CompactionReason,
) -> Option<CompactionArtifact> {
    use crate::context_window::estimate_text_tokens;

    let total_messages = messages.len();

    // 计算保留的最近 turn 起始索引（复用已有的函数）
    let keep_start = recent_turn_start_index(messages, keep_recent_turns).unwrap_or(total_messages);
    let messages_removed = keep_start;
    // hook 自定义摘要也必须真正替换掉一段旧历史；如果没有任何消息被折叠，
    // 继续生成 artifact 只会把“摘要 + 原始全量尾部”叠在一起，反而扩大上下文。
    if messages_removed == 0 {
        return None;
    }

    // 计算保留的 turn 数量
    let preserved_recent_turns = messages[keep_start..]
        .iter()
        .filter(|m| {
            matches!(
                m,
                astrcode_core::LlmMessage::User {
                    origin: astrcode_core::UserMessageOrigin::User,
                    ..
                }
            )
        })
        .count();

    // 估算 token 数
    let pre_tokens: usize = messages
        .iter()
        .map(|m| estimate_text_tokens(&format!("{:?}", m)))
        .sum();
    let summary_tokens = estimate_text_tokens(custom_summary);
    let post_tokens_estimate = summary_tokens
        + messages[keep_start..]
            .iter()
            .map(|m| estimate_text_tokens(&format!("{:?}", m)))
            .sum::<usize>();

    Some(CompactionArtifact {
        summary: custom_summary.to_string(),
        source_range: EventRange {
            start: 0,
            end: messages_removed,
        },
        preserved_tail_start: messages_removed as u64,
        strategy_id: "custom_summary_hook".to_string(),
        pre_tokens,
        post_tokens_estimate,
        compacted_at_seq: 0,
        trigger: reason,
        preserved_recent_turns,
        messages_removed,
        tokens_freed: pre_tokens.saturating_sub(post_tokens_estimate),
        recovered_files: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmMessage, StorageEvent, UserMessageOrigin};
    use astrcode_runtime_llm::{EventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits};

    use super::*;

    /// 测试用：总是返回一个静态 artifact 的压缩策略
    struct StaticArtifactStrategy;

    #[async_trait]
    impl CompactionStrategy for StaticArtifactStrategy {
        async fn compact(&self, _input: CompactionInput<'_>) -> Result<Option<CompactionArtifact>> {
            Ok(Some(CompactionArtifact {
                summary: "static summary".to_string(),
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
                recovered_files: Vec::new(),
            }))
        }
    }

    /// 测试用：总是失败的压缩策略
    struct FailingStrategy;

    #[async_trait]
    impl CompactionStrategy for FailingStrategy {
        async fn compact(&self, _input: CompactionInput<'_>) -> Result<Option<CompactionArtifact>> {
            Err(astrcode_core::AstrError::LlmStreamError(
                "test failure".to_string(),
            ))
        }
    }

    /// 测试用：空操作 LLM Provider
    struct NoopProvider;

    #[async_trait]
    impl LlmProvider for NoopProvider {
        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 100_000,
                max_output_tokens: 4_096,
            }
        }

        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<EventSink>,
        ) -> std::result::Result<LlmOutput, astrcode_core::AstrError> {
            Ok(LlmOutput {
                content: "noop".to_string(),
                tool_calls: Vec::new(),
                usage: None,
                reasoning: None,
                finish_reason: astrcode_runtime_llm::FinishReason::Stop,
            })
        }
    }

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
            recovered_files: Vec::new(),
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
            .rebuild(&artifact, &tail, &[])
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
            80,
            Arc::new(ThresholdCompactionPolicy::new(true)),
            Arc::new(AutoCompactStrategy),
            Arc::new(ConversationViewRebuilder),
            Arc::new(FsFileContentProvider),
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
            80,
            Arc::new(ThresholdCompactionPolicy::new(false)),
            Arc::new(AutoCompactStrategy),
            Arc::new(ConversationViewRebuilder),
            Arc::new(FsFileContentProvider),
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

    #[test]
    fn artifact_records_real_tail_storage_seq() {
        let mut artifact = CompactionArtifact {
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
            recovered_files: Vec::new(),
        };
        let tail = vec![
            StoredEvent {
                storage_seq: 7,
                event: StorageEvent::UserMessage {
                    turn_id: Some("turn-1".to_string()),
                    content: "older".to_string(),
                    origin: UserMessageOrigin::User,
                    timestamp: Utc::now(),
                },
            },
            StoredEvent {
                storage_seq: 11,
                event: StorageEvent::AssistantFinal {
                    turn_id: Some("turn-1".to_string()),
                    content: "latest".to_string(),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(Utc::now()),
                },
            },
        ];

        artifact.record_tail_seq(&tail);

        assert_eq!(artifact.compacted_at_seq, 11);
    }

    #[test]
    fn circuit_breaker_blocks_auto_compact_after_consecutive_failures() {
        let policy = ThresholdCompactionPolicy::new(true);
        let snapshot = PromptTokenSnapshot {
            context_tokens: 91,
            budget_tokens: 91,
            context_window: 100,
            effective_window: 90,
            threshold_tokens: 90,
        };

        // Initially should compact
        assert_eq!(
            policy.should_compact(&snapshot),
            Some(CompactionReason::Auto)
        );

        // After MAX failures, should not compact
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            policy.record_failure();
        }
        assert_eq!(policy.should_compact(&snapshot), None);

        // After success, should compact again
        policy.record_success();
        assert_eq!(
            policy.should_compact(&snapshot),
            Some(CompactionReason::Auto)
        );
    }

    #[test]
    fn circuit_breaker_does_not_fire_on_partial_failures() {
        let policy = ThresholdCompactionPolicy::new(true);
        let snapshot = PromptTokenSnapshot {
            context_tokens: 91,
            budget_tokens: 91,
            context_window: 100,
            effective_window: 90,
            threshold_tokens: 90,
        };

        // Two failures is still below the threshold
        policy.record_failure();
        policy.record_failure();
        assert_eq!(
            policy.should_compact(&snapshot),
            Some(CompactionReason::Auto)
        );
    }

    #[test]
    fn custom_summary_returns_none_when_keep_recent_turns_preserves_all_real_turns() {
        let messages = vec![
            LlmMessage::User {
                content: "turn-1".to_string(),
                origin: UserMessageOrigin::User,
            },
            LlmMessage::Assistant {
                content: "reply-1".to_string(),
                tool_calls: Vec::new(),
                reasoning: None,
            },
            LlmMessage::User {
                content: "turn-2".to_string(),
                origin: UserMessageOrigin::User,
            },
        ];

        let artifact = build_artifact_from_custom_summary(
            &messages,
            "custom summary",
            2,
            CompactionReason::Manual,
        );

        assert!(
            artifact.is_none(),
            "when no historical turn can be removed, custom summary should not fabricate a \
             compact artifact"
        );
    }

    #[tokio::test]
    async fn compact_records_success_and_resets_circuit_breaker() {
        let policy = Arc::new(ThresholdCompactionPolicy::new(true));
        // Pre-fail to 2
        policy.record_failure();
        policy.record_failure();

        let runtime = CompactionRuntime::new(
            true,
            1,
            80,
            policy.clone(),
            Arc::new(StaticArtifactStrategy),
            Arc::new(ConversationViewRebuilder),
            Arc::new(FsFileContentProvider),
        );

        let view = ConversationView::new(vec![]);
        let result = runtime
            .compact(
                &NoopProvider,
                &view,
                None,
                CompactionReason::Auto,
                astrcode_core::CancelToken::new(),
            )
            .await
            .expect("compact should succeed");

        assert!(result.is_some());
        // After success, circuit breaker should be reset
        let snapshot = PromptTokenSnapshot {
            context_tokens: 95,
            budget_tokens: 95,
            context_window: 100,
            effective_window: 90,
            threshold_tokens: 90,
        };
        assert_eq!(
            policy.should_compact(&snapshot),
            Some(CompactionReason::Auto)
        );
    }

    #[tokio::test]
    async fn compact_records_failure_for_auto_but_not_manual() {
        let policy = Arc::new(ThresholdCompactionPolicy::new(true));
        let runtime = CompactionRuntime::new(
            true,
            1,
            80,
            policy.clone(),
            Arc::new(FailingStrategy),
            Arc::new(ConversationViewRebuilder),
            Arc::new(FsFileContentProvider),
        );

        let view = ConversationView::new(vec![]);
        // Manual failure should NOT affect circuit breaker
        let _ = runtime
            .compact(
                &NoopProvider,
                &view,
                None,
                CompactionReason::Manual,
                astrcode_core::CancelToken::new(),
            )
            .await;

        let snapshot = PromptTokenSnapshot {
            context_tokens: 95,
            budget_tokens: 95,
            context_window: 100,
            effective_window: 90,
            threshold_tokens: 90,
        };
        assert_eq!(
            policy.should_compact(&snapshot),
            Some(CompactionReason::Auto),
            "manual failure should not affect circuit breaker"
        );

        // Auto failure SHOULD affect circuit breaker
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            let _ = runtime
                .compact(
                    &NoopProvider,
                    &view,
                    None,
                    CompactionReason::Auto,
                    astrcode_core::CancelToken::new(),
                )
                .await;
        }
        assert_eq!(
            policy.should_compact(&snapshot),
            None,
            "auto failures should trip circuit breaker"
        );
    }
}
