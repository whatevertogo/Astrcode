use std::fs;
use std::io::Write;

use chrono::Utc;

use crate::events::StorageEvent;
use crate::test_support::TestEnvGuard;

use super::*;

fn make_test_log(dir: &std::path::Path) -> EventLog {
    let session_id = "test-session-001";
    let path = dir.join(format!("session-{session_id}.jsonl"));
    EventLog::create_at_path(session_id, path).unwrap()
}

#[test]
fn append_and_load_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let mut log = make_test_log(tmp.path());

    let e1 = StorageEvent::SessionStart {
        session_id: "test-session-001".into(),
        timestamp: Utc::now(),
        working_dir: "/tmp".into(),
    };
    let e2 = StorageEvent::UserMessage {
        turn_id: None,
        content: "hello".into(),
        timestamp: Utc::now(),
    };

    log.append(&e1).unwrap();
    log.append(&e2).unwrap();

    let loaded = EventLog::load_from_path(log.path()).unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(
        matches!(&loaded[0].event, StorageEvent::SessionStart { session_id, .. } if session_id == "test-session-001")
    );
    assert!(
        matches!(&loaded[1].event, StorageEvent::UserMessage { content, .. } if content == "hello")
    );
}

#[test]
fn load_errors_on_invalid_json() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("session-bad.jsonl");
    {
        let mut f = File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"userMessage","content":"ok","timestamp":"2026-01-01T00:00:00Z"}}"#
        )
        .unwrap();
        writeln!(f, "THIS IS NOT JSON").unwrap();
    }
    let result = EventLog::load_from_path(&path);
    assert!(result.is_err());
}

#[test]
fn generate_session_id_format() {
    let id = generate_session_id();
    assert!(id.len() > 20);
    assert!(id.contains('T'));
    let parts: Vec<&str> = id.rsplitn(2, '-').collect();
    assert_eq!(parts[0].len(), 8);
}

#[test]
fn list_sessions_returns_sorted_ids() {
    let tmp = tempfile::tempdir().unwrap();

    let ids = [
        "2026-03-01T10-00-00-aaaaaaaa",
        "2026-03-02T12-30-00-bbbbbbbb",
        "2026-03-01T09-00-00-cccccccc",
    ];
    for id in &ids {
        let path = tmp.path().join(format!("session-{id}.jsonl"));
        let mut f = File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"sessionStart","sessionId":"{id}","timestamp":"2026-01-01T00:00:00Z","workingDir":"/tmp"}}"#).unwrap();
    }

    File::create(tmp.path().join("other-file.txt")).unwrap();
    File::create(tmp.path().join("not-session-123.jsonl")).unwrap();
    fs::create_dir(tmp.path().join("session-dir-inside.jsonl")).unwrap();

    let found = EventLog::list_sessions_from_path(tmp.path()).unwrap();

    assert_eq!(found.len(), 3);
    assert_eq!(found[0], "2026-03-01T09-00-00-cccccccc");
    assert_eq!(found[1], "2026-03-01T10-00-00-aaaaaaaa");
    assert_eq!(found[2], "2026-03-02T12-30-00-bbbbbbbb");
}

#[test]
fn session_path_normalizes_prefixed_ids() {
    let guard = TestEnvGuard::new();
    let path = session_path("session-2026-03-08T10-00-00-aaaaaaaa").unwrap();
    assert!(
        path.starts_with(guard.home_dir()),
        "session path should stay under the isolated test home"
    );
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        file_name == "session-2026-03-08T10-00-00-aaaaaaaa.jsonl",
        "actual file name: {file_name}"
    );
}

#[test]
fn list_sessions_handles_legacy_double_prefixed_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp
        .path()
        .join("session-session-2026-03-08T10-00-00-aaaaaaaa.jsonl");
    {
        let mut f = File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"userMessage","content":"legacy","timestamp":"2026-01-01T00:00:00Z"}}"#
        )
        .unwrap();
    }

    let found = EventLog::list_sessions_from_path(tmp.path()).unwrap();
    assert_eq!(found, vec!["session-2026-03-08T10-00-00-aaaaaaaa"]);
}

#[test]
fn list_sessions_with_meta_extracts_fields_and_sorts_by_updated_at() {
    let tmp = tempfile::tempdir().unwrap();
    let id_a = "2026-03-08T10-00-00-aaaaaaaa";
    let id_b = "2026-03-08T11-00-00-bbbbbbbb";
    let path_a = tmp.path().join(format!("session-{id_a}.jsonl"));
    let path_b = tmp.path().join(format!("session-{id_b}.jsonl"));

    {
        let mut file = File::create(&path_a).unwrap();
        let events = [
            StorageEvent::SessionStart {
                session_id: id_a.to_string(),
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T10:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                working_dir: r"D:\repo\a".to_string(),
            },
            StorageEvent::UserMessage {
                turn_id: None,
                content: "session-a-title".to_string(),
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T10:01:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T10:02:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
        ];
        for event in events {
            writeln!(file, "{}", serde_json::to_string(&event).unwrap()).unwrap();
        }
    }

    {
        let mut file = File::create(&path_b).unwrap();
        let events = [
            StorageEvent::SessionStart {
                session_id: id_b.to_string(),
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T11:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                working_dir: r"D:\repo\b".to_string(),
            },
            StorageEvent::UserMessage {
                turn_id: None,
                content: "session-b-title".to_string(),
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T11:01:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T11:02:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
        ];
        for event in events {
            writeln!(file, "{}", serde_json::to_string(&event).unwrap()).unwrap();
        }
    }

    let metas = EventLog::list_sessions_with_meta_from_path(tmp.path()).unwrap();
    assert_eq!(metas.len(), 2);
    assert_eq!(metas[0].session_id, id_b);
    assert_eq!(metas[1].session_id, id_a);
    assert_eq!(metas[0].title, "session-b-title");
    assert_eq!(metas[0].display_name, "b");
}

#[test]
fn delete_session_from_path_succeeds_and_missing_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let id = "2026-03-08T12-00-00-aaaaaaaa";
    let path = tmp.path().join(format!("session-{id}.jsonl"));
    File::create(&path).unwrap();

    EventLog::delete_session_from_path(tmp.path(), id).unwrap();
    assert!(!path.exists());
    assert!(EventLog::delete_session_from_path(tmp.path(), id).is_err());
}

#[test]
fn delete_sessions_by_working_dir_continues_on_partial_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let working_dir = r"D:\repo\alpha";
    let id_ok = "2026-03-08T13-00-00-aaaaaaaa";
    let id_fail = "session-2026-03-08T13-00-01-bbbbbbbb";

    let path_ok = tmp.path().join(format!("session-{id_ok}.jsonl"));
    let path_fail = tmp.path().join(format!("session-{id_fail}.jsonl"));

    for (id, path) in [(id_ok, &path_ok), (id_fail, &path_fail)] {
        let mut file = File::create(path).unwrap();
        let events = [
            StorageEvent::SessionStart {
                session_id: id.to_string(),
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T13:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                working_dir: working_dir.to_string(),
            },
            StorageEvent::TurnDone {
                turn_id: None,
                timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T13:05:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
        ];
        for event in events {
            writeln!(file, "{}", serde_json::to_string(&event).unwrap()).unwrap();
        }
    }

    let result =
        EventLog::delete_sessions_by_working_dir_from_path(tmp.path(), working_dir).unwrap();
    assert_eq!(result.success_count, 1);
    assert_eq!(
        result.failed_session_ids,
        vec!["2026-03-08T13-00-01-bbbbbbbb".to_string()]
    );
    assert!(!path_ok.exists());
}

#[test]
fn session_path_prefers_isolated_test_home_over_explicit_override() {
    let guard = TestEnvGuard::new();
    let override_home = tempfile::tempdir().unwrap();
    let previous_override = std::env::var_os("ASTRCODE_HOME_DIR");

    std::env::set_var("ASTRCODE_HOME_DIR", override_home.path());
    let path = session_path("2026-03-08T10-00-00-aaaaaaaa").unwrap();
    let uses_test_home = path.starts_with(guard.home_dir());

    match previous_override {
        Some(value) => std::env::set_var("ASTRCODE_HOME_DIR", value),
        None => std::env::remove_var("ASTRCODE_HOME_DIR"),
    }

    assert!(
        uses_test_home,
        "session path should stay under the isolated test home"
    );
}

#[test]
fn session_path_rejects_invalid_session_id() {
    let _guard = TestEnvGuard::new();
    let err = session_path("../../etc/passwd").expect_err("invalid id should fail");
    assert!(err.to_string().contains("invalid session id"));
}

#[test]
fn list_sessions_ignores_invalid_session_filenames() {
    let tmp = tempfile::tempdir().unwrap();
    let valid = tmp
        .path()
        .join("session-2026-03-10T10-00-00-aaaaaaaa.jsonl");
    let invalid = tmp.path().join("session-evil..id.jsonl");

    File::create(valid).unwrap();
    File::create(invalid).unwrap();

    let found = EventLog::list_sessions_from_path(tmp.path()).unwrap();
    assert_eq!(found, vec!["2026-03-10T10-00-00-aaaaaaaa"]);
}
