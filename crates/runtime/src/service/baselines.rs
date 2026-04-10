//! # 基线测试 (Baseline Tests)
//!
//! 验证运行时关键路径的行为和指标收集是否正确，包括：
//! - 长会话加载后的 prompt 提交不应触发重复水合
//! - SSE 重连时缓存命中 vs 磁盘回退的指标记录
//! - 并发提交时的分支会话行为
//!
//! ## 设计
//!
//! 使用静态 LLM Provider 模拟确定性输出，避免真实 API 调用的不确定性。
//! 通过 `seed_session_log` 生成预定义的 JSONL 会话日志，验证回放和指标逻辑。

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use astrcode_core::{
    AgentEventContext, AstrError, Result, StorageEvent, StorageEventPayload, StoredEvent,
    UserMessageOrigin,
};
use astrcode_runtime_agent_loop::{AgentLoop, ProviderFactory};
use astrcode_storage::session::EventLog;
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};

use super::RuntimeService;
use crate::{
    llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest, ModelLimits},
    test_support::{TestEnvGuard, empty_capabilities},
};

async fn latest_cursor(service: &Arc<RuntimeService>, session_id: &str) -> String {
    service
        .sessions()
        .history(session_id)
        .await
        .expect("history should load")
        .cursor
        .expect("history cursor should exist")
}

fn user_messages_from_history(history: &[astrcode_core::SessionEventRecord]) -> Vec<&str> {
    history
        .iter()
        .filter_map(|record| match &record.event {
            astrcode_core::AgentEvent::UserMessage { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect()
}

/// 静态 LLM Provider，返回预设输出并支持模拟延迟。
///
/// 用于测试中替代真实 LLM 调用，确保确定性和可重复性。
struct StaticProvider {
    delay: Duration,
    output: LlmOutput,
}

#[async_trait]
impl LlmProvider for StaticProvider {
    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 200_000,
            max_output_tokens: 4_096,
        }
    }

    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        // 支持取消检查，模拟真实 LLM 调用的中断行为
        if !self.delay.is_zero() {
            tokio::select! {
                _ = crate::llm::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
                _ = tokio::time::sleep(self.delay) => {}
            }
        }

        // 模拟流式输出，逐个字符推送 delta
        if let Some(sink) = sink {
            for delta in self.output.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));
            }
        }

        Ok(self.output.clone())
    }
}

/// 静态 Provider 工厂，始终返回同一个预配置的 Provider 实例。
struct StaticProviderFactory {
    provider: Arc<dyn LlmProvider>,
}

impl ProviderFactory for StaticProviderFactory {
    fn build_for_working_dir(&self, _working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>> {
        Ok(Arc::clone(&self.provider))
    }
}

/// 为测试服务安装一个静态 AgentLoop，使用固定延迟和输出。
async fn install_test_loop(service: &RuntimeService, delay: Duration) {
    let provider: Arc<dyn LlmProvider> = Arc::new(StaticProvider {
        delay,
        output: LlmOutput {
            content: "done".to_string(),
            ..LlmOutput::default()
        },
    });
    let loop_ = AgentLoop::from_capabilities(
        Arc::new(StaticProviderFactory { provider }),
        empty_capabilities(),
    );
    *service.loop_.write().await = Arc::new(loop_);
}

/// 为测试会话播种预定义的 JSONL 日志。
///
/// 生成包含指定数量 Turn 的完整会话历史，用于验证回放和指标逻辑。
fn seed_session_log(session_id: &str, working_dir: &Path, turns: usize) {
    let log = EventLog::create(session_id, working_dir).expect("session file should be created");
    let path = log.path().to_path_buf();
    drop(log);

    let file = File::create(&path).expect("session file should be writable");
    let mut writer = BufWriter::new(file);
    let started_at = Utc::now();
    let mut storage_seq = 1_u64;

    // 写入会话开始事件
    write_stored_event(
        &mut writer,
        &mut storage_seq,
        StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: session_id.to_string(),
                timestamp: started_at,
                working_dir: working_dir.display().to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
    );

    // 写入指定数量的 Turn 事件（UserMessage + AssistantFinal + TurnDone）
    for turn_index in 0..turns {
        let timestamp = started_at + ChronoDuration::seconds(turn_index as i64 + 1);
        let turn_id = format!("turn-{turn_index}");
        write_stored_event(
            &mut writer,
            &mut storage_seq,
            StorageEvent {
                turn_id: Some(turn_id.clone()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::UserMessage {
                    content: format!("prompt {turn_index}"),
                    origin: UserMessageOrigin::User,
                    timestamp,
                },
            },
        );
        write_stored_event(
            &mut writer,
            &mut storage_seq,
            StorageEvent {
                turn_id: Some(turn_id.clone()),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::AssistantFinal {
                    content: format!("response {turn_index}"),
                    reasoning_content: None,
                    reasoning_signature: None,
                    timestamp: Some(timestamp),
                },
            },
        );
        write_stored_event(
            &mut writer,
            &mut storage_seq,
            StorageEvent {
                turn_id: Some(turn_id),
                agent: AgentEventContext::default(),
                payload: StorageEventPayload::TurnDone {
                    timestamp,
                    reason: Some("completed".to_string()),
                },
            },
        );
    }

    writer.flush().expect("session file should flush");
    writer
        .get_ref()
        .sync_all()
        .expect("session file should sync");
}

/// 将单个存储事件序列化并写入 JSONL 文件。
fn write_stored_event(writer: &mut BufWriter<File>, storage_seq: &mut u64, event: StorageEvent) {
    serde_json::to_writer(
        &mut *writer,
        &StoredEvent {
            storage_seq: *storage_seq,
            event,
        },
    )
    .expect("stored event should serialize");
    writer
        .write_all(b"\n")
        .expect("stored event newline should write");
    *storage_seq += 1;
}

/// 等待所有会话变为空闲（无运行中的会话）。
async fn wait_until_idle(service: &RuntimeService) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if service.running_session_ids().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("session should become idle");
}

/// 验证长会话加载后提交 prompt 不会触发重复水合，并正确记录 Turn 指标。
///
/// 场景：加载包含 512 个 Turn 的会话，然后提交新 prompt。
/// 预期：水合计数不变（已加载的会话不应重新水合），Turn 执行计数 +1。
#[tokio::test]
async fn long_loaded_session_submit_avoids_extra_rehydrate_and_records_turn_metrics() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should initialize"),
    );
    install_test_loop(&service, Duration::from_millis(0)).await;

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_session_log("baseline-long-submit", working_dir.path(), 512);

    service
        .ensure_session_loaded("baseline-long-submit")
        .await
        .expect("long session should load");
    let before = service.observability().snapshot();
    assert_eq!(before.session_rehydrate.total, 1);

    service
        .execution()
        .submit_prompt("baseline-long-submit", "follow-up".to_string())
        .await
        .expect("prompt should be accepted");
    wait_until_idle(&service).await;

    let after = service.observability().snapshot();
    assert_eq!(
        after.session_rehydrate.total, before.session_rehydrate.total,
        "loaded sessions must not be rehydrated again during prompt submission"
    );
    assert_eq!(after.turn_execution.total, before.turn_execution.total + 1);
    assert_eq!(
        after.turn_execution.failures,
        before.turn_execution.failures
    );
}

/// 验证使用最新游标重连时命中缓存，不触发磁盘回放。
///
/// 场景：加载 64 个 Turn 的会话，使用最新游标重连。
/// 预期：回放历史为空（缓存命中），缓存命中计数 +1。
#[tokio::test]
async fn reconnect_with_recent_cursor_uses_cached_tail_and_updates_metrics() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should initialize"),
    );

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_session_log("baseline-replay-cache", working_dir.path(), 64);
    service
        .ensure_session_loaded("baseline-replay-cache")
        .await
        .expect("session should load");

    let cursor = latest_cursor(&service, "baseline-replay-cache").await;
    let before = service.observability().snapshot();
    let replay = service
        .sessions()
        .replay("baseline-replay-cache", Some(&cursor))
        .await
        .expect("replay should succeed");

    assert!(
        replay.history.is_empty(),
        "reconnecting from the latest cursor should not need disk replay"
    );

    let after = service.observability().snapshot();
    assert_eq!(
        after.sse_catch_up.cache_hits,
        before.sse_catch_up.cache_hits + 1
    );
    assert_eq!(
        after.sse_catch_up.disk_fallbacks, before.sse_catch_up.disk_fallbacks,
        "recent reconnect should stay on the cached tail"
    );
}

/// 验证使用过期游标重连时回退到磁盘回放，并正确记录指标。
///
/// 场景：加载 1500 个 Turn 的会话，使用过期游标 "1.0" 重连。
/// 预期：从磁盘恢复历史事件，磁盘回退计数 +1。
#[tokio::test]
async fn reconnect_with_stale_cursor_falls_back_to_disk_and_records_it() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should initialize"),
    );

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_session_log("baseline-replay-disk", working_dir.path(), 1_500);
    service
        .ensure_session_loaded("baseline-replay-disk")
        .await
        .expect("session should load");

    let before = service.observability().snapshot();
    let replay = service
        .sessions()
        .replay("baseline-replay-disk", Some("1.0"))
        .await
        .expect("replay should succeed");

    assert!(
        !replay.history.is_empty(),
        "stale cursors must recover history from durable storage"
    );

    let after = service.observability().snapshot();
    assert_eq!(
        after.sse_catch_up.disk_fallbacks,
        before.sse_catch_up.disk_fallbacks + 1
    );
    assert!(
        after.sse_catch_up.recovered_events
            >= before.sse_catch_up.recovered_events + replay.history.len() as u64
    );
}

/// 验证并发提交时第二个 prompt 会分支到新会话，并正确记录两个 Turn。
///
/// 场景：在第一个 Turn 运行中提交第二个 prompt。
/// 预期：第二个 prompt 分支到新会话，原始会话只包含 "first"，分支会话只包含 "second"。
#[tokio::test]
async fn concurrent_submit_branches_second_prompt_and_records_two_turns() {
    let _guard = TestEnvGuard::new();
    let service = Arc::new(
        RuntimeService::from_capabilities(empty_capabilities())
            .expect("runtime service should initialize"),
    );
    install_test_loop(&service, Duration::from_millis(250)).await;

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .sessions()
        .create(working_dir.path())
        .await
        .expect("session should be created");

    let first = service
        .execution()
        .submit_prompt(&session.session_id, "first".to_string())
        .await
        .expect("first prompt should be accepted");
    let second = service
        .execution()
        .submit_prompt(&session.session_id, "second".to_string())
        .await
        .expect("second prompt should branch while the first turn runs");
    assert_eq!(first.session_id, session.session_id);
    assert_eq!(first.branched_from_session_id, None);
    assert_ne!(second.session_id, session.session_id);
    assert_eq!(
        second.branched_from_session_id.as_deref(),
        Some(session.session_id.as_str())
    );

    wait_until_idle(&service).await;
    let original_history = service
        .sessions()
        .history(&session.session_id)
        .await
        .expect("original session history should load");
    let branched_history = service
        .sessions()
        .history(&second.session_id)
        .await
        .expect("branched session history should load");

    let original_user_messages = user_messages_from_history(&original_history.history);
    let branched_user_messages = user_messages_from_history(&branched_history.history);
    assert_eq!(original_user_messages, vec!["first"]);
    assert_eq!(branched_user_messages, vec!["second"]);

    let metrics = service.observability().snapshot();
    assert_eq!(metrics.turn_execution.total, 2);
    assert_eq!(metrics.turn_execution.failures, 0);
}
