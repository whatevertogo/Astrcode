//! # 存储事件类型
//!
//! 定义了持久化到 JSONL 日志中的事件格式。
//!
//! ## 与领域事件的区别
//!
//! `StorageEvent` 是面向存储的格式，直接序列化到 JSONL 文件；
//! `AgentEvent` 是面向 SSE 推送的格式，由 [`EventTranslator`](crate::event::EventTranslator)
//! 从 `StorageEvent` 转换而来。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgentEventContext, ChildSessionNotification, MailboxBatchAckedPayload,
    MailboxBatchStartedPayload, MailboxDiscardedPayload, MailboxQueuedPayload,
    ResolvedExecutionLimitsSnapshot, ResolvedSubagentContextOverrides, SubRunResult,
    ToolOutputStream, UserMessageOrigin,
};

/// Prompt/缓存指标共享载荷。
///
/// `StorageEvent::PromptMetrics` 和 `AgentEvent::PromptMetrics` 都通过此类型共享指标字段，
/// 避免三层逐字段复制导致漂移。
///
/// 注意：不使用 `rename_all`，因为 `StorageEvent` 的 tagged enum 会原样传递内联字段名，
/// 而 `AgentEvent` 使用 adjacently tagged 且字段嵌套在 `"data"` 键下。
/// 各层的 serde 上下文已处理好字段命名。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptMetricsPayload {
    pub step_index: u32,
    pub estimated_tokens: u32,
    pub context_window: u32,
    pub effective_window: u32,
    pub threshold_tokens: u32,
    pub truncated_tool_results: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    pub provider_cache_metrics_supported: bool,
    #[serde(default)]
    pub prompt_cache_reuse_hits: u32,
    #[serde(default)]
    pub prompt_cache_reuse_misses: u32,
}

/// 上下文压缩的触发方式。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactTrigger {
    /// 自动触发（上下文窗口接近阈值时）
    Auto,
    /// 手动触发（用户主动请求）
    Manual,
}

/// 存储事件载荷。
///
/// 只描述事件本体，不包含跨变体共享的头部字段（`turn_id` 与 `agent`）。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StorageEventPayload {
    /// 会话启动事件。
    ///
    /// 包含 `parent_session_id` 和 `parent_storage_seq` 用于支持 session 分叉。
    SessionStart {
        session_id: String,
        #[serde(with = "crate::local_rfc3339")]
        timestamp: DateTime<Utc>,
        working_dir: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_storage_seq: Option<u64>,
    },
    /// 用户输入消息。
    UserMessage {
        content: String,
        #[serde(with = "crate::local_rfc3339")]
        timestamp: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "is_default_user_message_origin")]
        origin: UserMessageOrigin,
    },
    /// LLM 文本输出增量（流式响应片段）。
    AssistantDelta { token: String },
    /// LLM 推理/思考内容增量。
    ThinkingDelta { token: String },
    /// LLM 助手最终回复（完整内容）。
    AssistantFinal {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_signature: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::local_rfc3339_option"
        )]
        timestamp: Option<DateTime<Utc>>,
    },
    /// 工具调用开始。
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    /// 工具流式输出增量。
    ToolCallDelta {
        tool_call_id: String,
        #[serde(default)]
        tool_name: String,
        stream: ToolOutputStream,
        delta: String,
    },
    /// 工具调用完成结果。
    ToolResult {
        tool_call_id: String,
        #[serde(default)]
        tool_name: String,
        output: String,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<Value>,
        duration_ms: u64,
    },
    /// 上下文窗口指标快照（用于监控和压缩决策）。
    PromptMetrics {
        #[serde(flatten)]
        metrics: PromptMetricsPayload,
    },
    /// 上下文压缩已应用。
    CompactApplied {
        trigger: CompactTrigger,
        summary: String,
        preserved_recent_turns: u32,
        pre_tokens: u32,
        post_tokens_estimate: u32,
        messages_removed: u32,
        tokens_freed: u32,
        #[serde(with = "crate::local_rfc3339")]
        timestamp: DateTime<Utc>,
    },
    /// 受控子会话开始执行。
    SubRunStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        resolved_overrides: ResolvedSubagentContextOverrides,
        resolved_limits: ResolvedExecutionLimitsSnapshot,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::local_rfc3339_option"
        )]
        timestamp: Option<DateTime<Utc>>,
    },
    /// 受控子会话执行结束。
    SubRunFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        result: SubRunResult,
        step_count: u32,
        estimated_tokens: u64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::local_rfc3339_option"
        )]
        timestamp: Option<DateTime<Utc>>,
    },
    /// 子会话通知事件（父会话摘要投影的 durable 来源）。
    ChildSessionNotification {
        notification: ChildSessionNotification,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::local_rfc3339_option"
        )]
        timestamp: Option<DateTime<Utc>>,
    },
    /// Turn 完成（一轮 Agent 循环结束）。
    TurnDone {
        #[serde(with = "crate::local_rfc3339")]
        timestamp: DateTime<Utc>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Durable mailbox 消息入队。
    ///
    /// 记录一条协作消息成功进入目标 agent 的 mailbox。
    /// live inbox 只能在该事件 append 成功后更新。
    AgentMailboxQueued {
        #[serde(flatten)]
        payload: MailboxQueuedPayload,
    },
    /// Mailbox 批次开始消费。
    ///
    /// snapshot drain 时写入，记录本轮接管了哪些 delivery_ids。
    /// 必须是 mailbox-wake turn 的第一条 durable 事件。
    AgentMailboxBatchStarted {
        #[serde(flatten)]
        payload: MailboxBatchStartedPayload,
    },
    /// Mailbox 批次确认完成。
    ///
    /// durable turn completion 后写入，标记对应 delivery_ids 已被消费。
    AgentMailboxBatchAcked {
        #[serde(flatten)]
        payload: MailboxBatchAckedPayload,
    },
    /// Mailbox 消息丢弃。
    ///
    /// close 时写入，记录被主动丢弃的 pending delivery_ids。
    AgentMailboxDiscarded {
        #[serde(flatten)]
        payload: MailboxDiscardedPayload,
    },
    /// 错误事件。
    Error {
        message: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "crate::local_rfc3339_option"
        )]
        timestamp: Option<DateTime<Utc>>,
    },
}

/// 存储事件。
///
/// 这是 append-only JSONL 日志中的事件格式，每种 payload 代表 Agent 运行时的一个关键动作。
/// 与 `AgentEvent` 不同，`StorageEvent` 是持久化格式，不直接面向前端展示。
///
/// ## 事件生命周期
///
/// 1. 运行时产生事件 → 2. 通过 `EventLogWriter::append` 持久化 →
/// 3. 通过 `EventTranslator` 转换为 `AgentEvent` → 4. SSE 推送到前端
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StorageEvent {
    /// turn 级别关联 ID。SessionStart 没有该字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Agent 谱系上下文。SessionStart 默认为空且不序列化。
    #[serde(default, flatten, skip_serializing_if = "AgentEventContext::is_empty")]
    pub agent: AgentEventContext,
    /// 事件具体载荷。
    #[serde(flatten)]
    pub payload: StorageEventPayload,
}

impl StorageEvent {
    /// 提取事件关联的 turn ID（如果存在）。
    ///
    /// `SessionStart` 没有 turn_id，返回 `None`。
    pub fn turn_id(&self) -> Option<&str> {
        if self.is_session_start() {
            None
        } else {
            self.turn_id.as_deref()
        }
    }

    /// 提取 turn 事件附带的 Agent 元数据。
    pub fn agent_context(&self) -> Option<&AgentEventContext> {
        if self.is_session_start() {
            None
        } else {
            Some(&self.agent)
        }
    }

    fn is_session_start(&self) -> bool {
        matches!(self.payload, StorageEventPayload::SessionStart { .. })
    }
}

fn is_default_user_message_origin(origin: &UserMessageOrigin) -> bool {
    matches!(origin, UserMessageOrigin::User)
}

/// 已持久化的存储事件。
///
/// 包含单调递增的 `storage_seq`（由会话 writer 独占分配）和实际的事件内容。
/// `storage_seq` 用于 SSE 断点续传和事件排序。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    /// 存储序号，单调递增，由会话 writer 独占分配
    pub storage_seq: u64,
    /// 实际的事件内容
    #[serde(flatten)]
    pub event: StorageEvent,
}

/// JSONL 日志行的反序列化包装。
///
/// 支持两种格式：
/// - `Stored`: 新格式，包含 `storage_seq` 字段
/// - `Legacy`: 旧格式，没有 `storage_seq`，需要回退分配
///
/// 注意：Legacy 变体仅用于解析已有旧格式文件，新写入的事件始终使用 Stored 格式。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StoredEventLine {
    /// 新格式（包含 storage_seq）
    /// untagged 的匹配顺序是"先 Stored 后 Legacy"，依赖 storage_seq 字段是否存在来区分。
    /// Fixme:这在目前是对的，但如果未来 StorageEvent 里某个变体恰好也有 storage_seq
    /// 字段，会静默匹配错误
    Stored(StoredEvent),
    /// 旧格式（没有 storage_seq）、
    /// TODO:需要去除旧兼容
    Legacy(StorageEvent),
}

impl StoredEventLine {
    /// 将日志行转换为 `StoredEvent`。
    ///
    /// 新格式直接使用；旧格式使用 `fallback_seq` 作为 `storage_seq`。
    pub fn into_stored(self, fallback_seq: u64) -> StoredEvent {
        match self {
            Self::Stored(stored) => stored,
            Self::Legacy(event) => StoredEvent {
                storage_seq: fallback_seq,
                event,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::Value;

    use super::{CompactTrigger, PromptMetricsPayload, StorageEvent, StorageEventPayload};
    use crate::{
        AgentEventContext, AgentLifecycleStatus, ResolvedExecutionLimitsSnapshot,
        ResolvedSubagentContextOverrides, SubRunResult, SubRunStorageMode, format_local_rfc3339,
    };

    #[test]
    fn tool_result_deserializes_legacy_lines_without_error_or_metadata() {
        let event: StorageEvent = serde_json::from_str(
            r#"{"type":"toolResult","turn_id":"turn-1","tool_call_id":"call-1","tool_name":"readFile","output":"hello","success":true,"duration_ms":12}"#,
        )
        .expect("legacy tool result should deserialize");

        match event {
            StorageEvent {
                payload:
                    StorageEventPayload::ToolResult {
                        error, metadata, ..
                    },
                ..
            } => {
                assert_eq!(error, None);
                assert_eq!(metadata, None);
            },
            other => panic!("expected tool result, got {other:?}"),
        }
    }

    #[test]
    fn turn_done_deserializes_legacy_lines_without_reason() {
        let event: StorageEvent = serde_json::from_str(
            r#"{"type":"turnDone","turn_id":"turn-1","timestamp":"2026-01-01T00:00:00Z"}"#,
        )
        .expect("legacy turn done should deserialize");

        match event {
            StorageEvent {
                payload: StorageEventPayload::TurnDone { reason, .. },
                ..
            } => {
                assert_eq!(reason, None);
            },
            other => panic!("expected turn done, got {other:?}"),
        }
    }

    #[test]
    fn prompt_metrics_round_trip_preserves_all_fields() {
        let event = StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::PromptMetrics {
                metrics: PromptMetricsPayload {
                    step_index: 2,
                    estimated_tokens: 1_234,
                    context_window: 128_000,
                    effective_window: 108_000,
                    threshold_tokens: 97_200,
                    truncated_tool_results: 3,
                    provider_input_tokens: Some(800),
                    provider_output_tokens: Some(120),
                    cache_creation_input_tokens: Some(600),
                    cache_read_input_tokens: Some(500),
                    provider_cache_metrics_supported: true,
                    prompt_cache_reuse_hits: 2,
                    prompt_cache_reuse_misses: 1,
                },
            },
        };

        let encoded = serde_json::to_value(&event).expect("event should serialize");
        let decoded: StorageEvent =
            serde_json::from_value(encoded.clone()).expect("event should deserialize");

        match decoded {
            StorageEvent {
                turn_id,
                payload: StorageEventPayload::PromptMetrics { metrics },
                ..
            } => {
                assert_eq!(turn_id.as_deref(), Some("turn-1"));
                assert_eq!(metrics.step_index, 2);
                assert_eq!(metrics.estimated_tokens, 1_234);
                assert_eq!(metrics.context_window, 128_000);
                assert_eq!(metrics.effective_window, 108_000);
                assert_eq!(metrics.threshold_tokens, 97_200);
                assert_eq!(metrics.truncated_tool_results, 3);
                assert_eq!(metrics.provider_input_tokens, Some(800));
                assert_eq!(metrics.provider_output_tokens, Some(120));
                assert_eq!(metrics.cache_creation_input_tokens, Some(600));
                assert_eq!(metrics.cache_read_input_tokens, Some(500));
                assert!(metrics.provider_cache_metrics_supported);
                assert_eq!(metrics.prompt_cache_reuse_hits, 2);
                assert_eq!(metrics.prompt_cache_reuse_misses, 1);
            },
            other => panic!("expected prompt metrics, got {other:?}"),
        }

        let expected: Value = serde_json::json!({
            "type": "promptMetrics",
            "turn_id": "turn-1",
            "step_index": 2,
            "estimated_tokens": 1234,
            "context_window": 128000,
            "effective_window": 108000,
            "threshold_tokens": 97200,
            "truncated_tool_results": 3,
            "provider_input_tokens": 800,
            "provider_output_tokens": 120,
            "cache_creation_input_tokens": 600,
            "cache_read_input_tokens": 500,
            "provider_cache_metrics_supported": true,
            "prompt_cache_reuse_hits": 2,
            "prompt_cache_reuse_misses": 1,
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn compact_applied_round_trip_preserves_all_fields() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("timestamp should build");
        let event = StorageEvent {
            turn_id: Some("turn-2".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::CompactApplied {
                trigger: CompactTrigger::Manual,
                summary: "condensed work".to_string(),
                preserved_recent_turns: 2,
                pre_tokens: 2_000,
                post_tokens_estimate: 600,
                messages_removed: 5,
                tokens_freed: 1_400,
                timestamp,
            },
        };

        let encoded = serde_json::to_value(&event).expect("event should serialize");
        let decoded: StorageEvent =
            serde_json::from_value(encoded.clone()).expect("event should deserialize");

        match decoded {
            StorageEvent {
                turn_id,
                payload:
                    StorageEventPayload::CompactApplied {
                        trigger,
                        summary,
                        preserved_recent_turns,
                        pre_tokens,
                        post_tokens_estimate,
                        messages_removed,
                        tokens_freed,
                        timestamp: decoded_timestamp,
                    },
                ..
            } => {
                assert_eq!(turn_id.as_deref(), Some("turn-2"));
                assert_eq!(trigger, CompactTrigger::Manual);
                assert_eq!(summary, "condensed work");
                assert_eq!(preserved_recent_turns, 2);
                assert_eq!(pre_tokens, 2_000);
                assert_eq!(post_tokens_estimate, 600);
                assert_eq!(messages_removed, 5);
                assert_eq!(tokens_freed, 1_400);
                assert_eq!(decoded_timestamp, timestamp);
            },
            other => panic!("expected compact applied, got {other:?}"),
        }

        assert_eq!(
            encoded,
            serde_json::json!({
                "type": "compactApplied",
                "turn_id": "turn-2",
                "trigger": "manual",
                "summary": "condensed work",
                "preserved_recent_turns": 2,
                "pre_tokens": 2000,
                "post_tokens_estimate": 600,
                "messages_removed": 5,
                "tokens_freed": 1400,
                "timestamp": format_local_rfc3339(timestamp),
            })
        );
    }

    #[test]
    fn session_start_serializes_timestamp_in_local_timezone_format() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("timestamp should build");
        let encoded = serde_json::to_value(StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: "session-1".to_string(),
                timestamp,
                working_dir: "/tmp/project".to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        })
        .expect("event should serialize");

        assert_eq!(
            encoded["timestamp"],
            Value::String(format_local_rfc3339(timestamp))
        );
    }

    #[test]
    fn subrun_lifecycle_round_trip_preserves_tool_call_id() {
        let started = StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: AgentEventContext::sub_run(
                "agent-child",
                "turn-parent",
                "review",
                "subrun-1",
                SubRunStorageMode::IndependentSession,
                Some("child-session".to_string()),
            ),
            payload: StorageEventPayload::SubRunStarted {
                tool_call_id: Some("call-1".to_string()),
                resolved_overrides: ResolvedSubagentContextOverrides::default(),
                resolved_limits: ResolvedExecutionLimitsSnapshot::default(),
                timestamp: None,
            },
        };
        let finished = StorageEvent {
            turn_id: Some("turn-parent".to_string()),
            agent: AgentEventContext::sub_run(
                "agent-child",
                "turn-parent",
                "review",
                "subrun-1",
                SubRunStorageMode::IndependentSession,
                Some("child-session".to_string()),
            ),
            payload: StorageEventPayload::SubRunFinished {
                tool_call_id: Some("call-1".to_string()),
                result: SubRunResult {
                    lifecycle: AgentLifecycleStatus::Idle,
                    last_turn_outcome: Some(crate::AgentTurnOutcome::Completed),
                    handoff: None,
                    failure: None,
                },
                step_count: 3,
                estimated_tokens: 99,
                timestamp: None,
            },
        };

        for event in [started, finished] {
            let encoded = serde_json::to_value(&event).expect("event should serialize");
            let decoded: StorageEvent =
                serde_json::from_value(encoded.clone()).expect("event should deserialize");

            match decoded {
                StorageEvent {
                    payload: StorageEventPayload::SubRunStarted { tool_call_id, .. },
                    ..
                }
                | StorageEvent {
                    payload: StorageEventPayload::SubRunFinished { tool_call_id, .. },
                    ..
                } => {
                    assert_eq!(tool_call_id.as_deref(), Some("call-1"));
                },
                other => panic!("expected subrun lifecycle event, got {other:?}"),
            }

            assert_eq!(encoded["tool_call_id"], Value::String("call-1".to_string()));
        }
    }

    // ─── T041 谱系兼容性测试 ──────────────────────────────

    /// 验证 spawn/fork/resume 三种 lineage kind 在 ChildAgentRef 中均可序列化/反序列化，
    /// 且在 durable 事件中正确传播。
    #[test]
    fn child_agent_ref_lineage_kind_spawn_fork_resume_all_roundtrip() {
        for (label, kind) in [
            ("spawn", crate::ChildSessionLineageKind::Spawn),
            ("fork", crate::ChildSessionLineageKind::Fork),
            ("resume", crate::ChildSessionLineageKind::Resume),
        ] {
            let child_ref = crate::ChildAgentRef {
                agent_id: "agent-child".to_string(),
                session_id: "session-parent".to_string(),
                sub_run_id: "subrun-1".to_string(),
                parent_agent_id: Some("agent-parent".to_string()),
                lineage_kind: kind,
                status: crate::AgentLifecycleStatus::Running,
                open_session_id: "session-child".to_string(),
            };

            let json = serde_json::to_value(&child_ref).expect("serialize child ref");
            assert_eq!(
                json.get("lineageKind"),
                Some(&serde_json::json!(label)),
                "lineage_kind {label} should serialize as snake_case"
            );

            let back: crate::ChildAgentRef =
                serde_json::from_value(json).expect("deserialize child ref");
            assert_eq!(
                back.lineage_kind, kind,
                "roundtrip for {label} should match"
            );
        }
    }

    /// 验证 ChildSessionNode 携带 fork/resume lineage 时在 durable 事件中正确传播。
    #[test]
    fn child_session_node_lineage_kind_preserves_fork_and_resume_in_durable_events() {
        for kind in [
            crate::ChildSessionLineageKind::Fork,
            crate::ChildSessionLineageKind::Resume,
        ] {
            let node = crate::ChildSessionNode {
                agent_id: "agent-child".to_string(),
                session_id: "session-parent".to_string(),
                child_session_id: "session-child".to_string(),
                sub_run_id: "subrun-1".to_string(),
                parent_session_id: "session-parent".to_string(),
                parent_agent_id: Some("agent-parent".to_string()),
                parent_turn_id: "turn-1".to_string(),
                lineage_kind: kind,
                status: crate::AgentLifecycleStatus::Idle,
                status_source: crate::ChildSessionStatusSource::Durable,
                created_by_tool_call_id: None,
                lineage_snapshot: None,
            };

            let json = serde_json::to_value(&node).expect("serialize node");
            let back: crate::ChildSessionNode =
                serde_json::from_value(json).expect("deserialize node");
            assert_eq!(back.lineage_kind, kind);

            // 验证 child_ref() 方法也正确传播 lineage
            let child_ref = node.child_ref();
            assert_eq!(child_ref.lineage_kind, kind);
        }
    }
}
