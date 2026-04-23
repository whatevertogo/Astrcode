use std::{fs, path::Path};

use astrcode_core::{
    AgentEventContext, StorageEvent, StorageEventPayload, StoredEvent, UserMessageOrigin,
};
use astrcode_eval::trace::extractor::TraceExtractor;
use chrono::{TimeZone, Utc};
use tempfile::tempdir;

fn append_event(path: &Path, storage_seq: u64, event: StorageEvent) {
    let line = serde_json::to_string(&StoredEvent { storage_seq, event })
        .expect("stored event should serialize");
    let existing = fs::read_to_string(path).unwrap_or_default();
    let content = if existing.is_empty() {
        format!("{line}\n")
    } else {
        format!("{existing}{line}\n")
    };
    fs::write(path, content).expect("fixture file should write");
}

#[test]
fn extractor_reads_session_jsonl_file_from_disk() {
    let temp_dir = tempdir().expect("tempdir should create");
    let jsonl_path = temp_dir.path().join("session-eval.jsonl");

    append_event(
        &jsonl_path,
        1,
        StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::SessionStart {
                session_id: "session-eval".to_string(),
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap(),
                working_dir: "D:/workspace".to_string(),
                parent_session_id: None,
                parent_storage_seq: None,
            },
        },
    );
    append_event(
        &jsonl_path,
        2,
        StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::root_execution("agent-root", "default"),
            payload: StorageEventPayload::UserMessage {
                content: "read README".to_string(),
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 1).unwrap(),
                origin: UserMessageOrigin::User,
            },
        },
    );
    append_event(
        &jsonl_path,
        3,
        StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::root_execution("agent-root", "default"),
            payload: StorageEventPayload::TurnDone {
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 2).unwrap(),
                terminal_kind: None,
                reason: Some("completed".to_string()),
            },
        },
    );
    append_event(
        &jsonl_path,
        4,
        StorageEvent {
            turn_id: Some("turn-2".to_string()),
            agent: AgentEventContext::root_execution("agent-root", "default"),
            payload: StorageEventPayload::UserMessage {
                content: "edit src/lib.rs".to_string(),
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 1, 0).unwrap(),
                origin: UserMessageOrigin::User,
            },
        },
    );
    append_event(
        &jsonl_path,
        5,
        StorageEvent {
            turn_id: Some("turn-2".to_string()),
            agent: AgentEventContext::root_execution("agent-root", "default"),
            payload: StorageEventPayload::TurnDone {
                timestamp: Utc.with_ymd_and_hms(2026, 4, 20, 8, 1, 5).unwrap(),
                terminal_kind: None,
                reason: Some("completed".to_string()),
            },
        },
    );

    let trace = TraceExtractor::extract_file(&jsonl_path).expect("extract should succeed");
    assert_eq!(trace.turns.len(), 2);
    assert_eq!(trace.turns[0].turn_id, "turn-1");
    assert_eq!(trace.turns[1].turn_id, "turn-2");
}
