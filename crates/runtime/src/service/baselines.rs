use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};

use astrcode_core::{AstrError, Result, StorageEvent, StoredEvent};
use astrcode_storage::session::EventLog;

use crate::agent_loop::AgentLoop;
use crate::llm::{EventSink, LlmEvent, LlmOutput, LlmProvider, LlmRequest};
use crate::provider_factory::ProviderFactory;
use crate::test_support::{empty_capabilities, TestEnvGuard};

use super::{RuntimeService, SessionReplaySource};

struct StaticProvider {
    delay: Duration,
    output: LlmOutput,
}

#[async_trait]
impl LlmProvider for StaticProvider {
    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        if !self.delay.is_zero() {
            tokio::select! {
                _ = crate::llm::cancelled(request.cancel.clone()) => return Err(AstrError::LlmInterrupted),
                _ = tokio::time::sleep(self.delay) => {}
            }
        }

        if let Some(sink) = sink {
            for delta in self.output.content.chars() {
                sink(LlmEvent::TextDelta(delta.to_string()));
            }
        }

        Ok(self.output.clone())
    }
}

struct StaticProviderFactory {
    provider: Arc<dyn LlmProvider>,
}

impl ProviderFactory for StaticProviderFactory {
    fn build_for_working_dir(&self, _working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>> {
        Ok(Arc::clone(&self.provider))
    }
}

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

fn seed_session_log(session_id: &str, working_dir: &Path, turns: usize) {
    let log = EventLog::create(session_id).expect("session file should be created");
    let path = log.path().to_path_buf();
    drop(log);

    let file = File::create(&path).expect("session file should be writable");
    let mut writer = BufWriter::new(file);
    let started_at = Utc::now();
    let mut storage_seq = 1_u64;

    write_stored_event(
        &mut writer,
        &mut storage_seq,
        StorageEvent::SessionStart {
            session_id: session_id.to_string(),
            timestamp: started_at,
            working_dir: working_dir.display().to_string(),
        },
    );

    for turn_index in 0..turns {
        let timestamp = started_at + ChronoDuration::seconds(turn_index as i64 + 1);
        let turn_id = format!("turn-{turn_index}");
        write_stored_event(
            &mut writer,
            &mut storage_seq,
            StorageEvent::UserMessage {
                turn_id: Some(turn_id.clone()),
                content: format!("prompt {turn_index}"),
                timestamp,
            },
        );
        write_stored_event(
            &mut writer,
            &mut storage_seq,
            StorageEvent::AssistantFinal {
                turn_id: Some(turn_id.clone()),
                content: format!("response {turn_index}"),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: Some(timestamp),
            },
        );
        write_stored_event(
            &mut writer,
            &mut storage_seq,
            StorageEvent::TurnDone {
                turn_id: Some(turn_id),
                timestamp,
            },
        );
    }

    writer.flush().expect("session file should flush");
    writer
        .get_ref()
        .sync_all()
        .expect("session file should sync");
}

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

#[tokio::test]
async fn long_loaded_session_submit_avoids_extra_rehydrate_and_records_turn_metrics() {
    let _guard = TestEnvGuard::new();
    let service = RuntimeService::from_capabilities(empty_capabilities())
        .expect("runtime service should initialize");
    install_test_loop(&service, Duration::from_millis(0)).await;

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_session_log("baseline-long-submit", working_dir.path(), 512);

    service
        .ensure_session_loaded("baseline-long-submit")
        .await
        .expect("long session should load");
    let before = service.observability_snapshot();
    assert_eq!(before.session_rehydrate.total, 1);

    service
        .submit_prompt("baseline-long-submit", "follow-up".to_string())
        .await
        .expect("prompt should be accepted");
    wait_until_idle(&service).await;

    let after = service.observability_snapshot();
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

#[tokio::test]
async fn reconnect_with_recent_cursor_uses_cached_tail_and_updates_metrics() {
    let _guard = TestEnvGuard::new();
    let service = RuntimeService::from_capabilities(empty_capabilities())
        .expect("runtime service should initialize");

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_session_log("baseline-replay-cache", working_dir.path(), 64);
    service
        .ensure_session_loaded("baseline-replay-cache")
        .await
        .expect("session should load");

    let (_, cursor) = service
        .load_session_snapshot("baseline-replay-cache")
        .await
        .expect("snapshot should load");
    let before = service.observability_snapshot();
    let replay = service
        .replay("baseline-replay-cache", cursor.as_deref())
        .await
        .expect("replay should succeed");

    assert!(
        replay.history.is_empty(),
        "reconnecting from the latest cursor should not need disk replay"
    );

    let after = service.observability_snapshot();
    assert_eq!(
        after.sse_catch_up.cache_hits,
        before.sse_catch_up.cache_hits + 1
    );
    assert_eq!(
        after.sse_catch_up.disk_fallbacks, before.sse_catch_up.disk_fallbacks,
        "recent reconnect should stay on the cached tail"
    );
}

#[tokio::test]
async fn reconnect_with_stale_cursor_falls_back_to_disk_and_records_it() {
    let _guard = TestEnvGuard::new();
    let service = RuntimeService::from_capabilities(empty_capabilities())
        .expect("runtime service should initialize");

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    seed_session_log("baseline-replay-disk", working_dir.path(), 1_500);
    service
        .ensure_session_loaded("baseline-replay-disk")
        .await
        .expect("session should load");

    let before = service.observability_snapshot();
    let replay = service
        .replay("baseline-replay-disk", Some("1.0"))
        .await
        .expect("replay should succeed");

    assert!(
        !replay.history.is_empty(),
        "stale cursors must recover history from durable storage"
    );

    let after = service.observability_snapshot();
    assert_eq!(
        after.sse_catch_up.disk_fallbacks,
        before.sse_catch_up.disk_fallbacks + 1
    );
    assert!(
        after.sse_catch_up.recovered_events
            >= before.sse_catch_up.recovered_events + replay.history.len() as u64
    );
}

#[tokio::test]
async fn concurrent_submit_rejects_second_prompt_and_records_single_turn() {
    let _guard = TestEnvGuard::new();
    let service = RuntimeService::from_capabilities(empty_capabilities())
        .expect("runtime service should initialize");
    install_test_loop(&service, Duration::from_millis(250)).await;

    let working_dir = tempfile::tempdir().expect("tempdir should be created");
    let session = service
        .create_session(working_dir.path())
        .await
        .expect("session should be created");

    service
        .submit_prompt(&session.session_id, "first".to_string())
        .await
        .expect("first prompt should be accepted");
    let second = service
        .submit_prompt(&session.session_id, "second".to_string())
        .await
        .expect_err("second prompt should be rejected while the first turn runs");
    assert!(
        second.to_string().contains("is already running"),
        "concurrent submission must fail with a running-session conflict"
    );

    wait_until_idle(&service).await;
    let metrics = service.observability_snapshot();
    assert_eq!(metrics.turn_execution.total, 1);
    assert_eq!(metrics.turn_execution.failures, 0);
}
