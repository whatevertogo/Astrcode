use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::StorageEvent;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub session_id: String,
    pub working_dir: String,
    pub display_name: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectResult {
    pub success_count: usize,
    pub failed_session_ids: Vec<String>,
}

pub struct EventLog {
    session_id: String,
    path: PathBuf,
    writer: BufWriter<File>,
}

/// Generate a new session id: `{datetime}-{uuid_short}`.
/// Example: `2026-03-08T12-30-01-a3f2b1c0`
pub fn generate_session_id() -> String {
    let dt = Utc::now().format("%Y-%m-%dT%H-%M-%S");
    let uuid = Uuid::new_v4().simple().to_string();
    let short = &uuid[..8];
    format!("{dt}-{short}")
}

fn sessions_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory"))?;
    Ok(home.join(".astrcode").join("sessions"))
}

fn canonical_session_id(session_id: &str) -> &str {
    session_id
        .strip_prefix("session-")
        .unwrap_or(session_id)
}

fn session_path(session_id: &str) -> Result<PathBuf> {
    let session_id = canonical_session_id(session_id);
    Ok(sessions_dir()?.join(format!("session-{session_id}.jsonl")))
}

fn legacy_prefixed_path(session_id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("session-{session_id}.jsonl")))
}

fn resolve_existing_session_path(session_id: &str) -> Result<PathBuf> {
    let canonical = session_path(session_id)?;
    if canonical.exists() {
        return Ok(canonical);
    }

    // Compatibility fallback for historical buggy IDs that already include "session-".
    let legacy = legacy_prefixed_path(session_id)?;
    if legacy != canonical && legacy.exists() {
        return Ok(legacy);
    }

    Err(anyhow!("session file not found: {}", canonical.display()))
}

fn timestamp_of_event(event: &StorageEvent) -> Option<DateTime<Utc>> {
    match event {
        StorageEvent::SessionStart { timestamp, .. } => Some(*timestamp),
        StorageEvent::UserMessage { timestamp, .. } => Some(*timestamp),
        StorageEvent::TurnDone { timestamp } => Some(*timestamp),
        _ => None,
    }
}

fn session_display_name(working_dir: &str) -> String {
    let normalized = working_dir.trim_end_matches(['/', '\\']);
    normalized
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or("默认项目")
        .to_string()
}

fn title_from_user_message(content: &str) -> String {
    let title: String = content.chars().take(20).collect();
    let title = title.trim();
    if title.is_empty() {
        "新会话".to_string()
    } else {
        title.to_string()
    }
}

impl EventLog {
    /// Create a new session log at an arbitrary path (for testing).
    #[cfg(test)]
    pub fn create_at_path(session_id: &str, path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create directory: {}", parent.display())
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to create session file: {}", path.display()))?;
        Ok(Self {
            session_id: session_id.to_string(),
            path,
            writer: BufWriter::new(file),
        })
    }

    /// Create a new session log file. Creates parent directories if needed.
    /// Returns error if the file already exists.
    pub fn create(session_id: &str) -> Result<Self> {
        let path = session_path(session_id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create sessions directory: {}", parent.display())
            })?;
        }
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to create session file: {}", path.display()))?;
        Ok(Self {
            session_id: session_id.to_string(),
            path,
            writer: BufWriter::new(file),
        })
    }

    /// Open an existing session log file.
    pub fn open(session_id: &str) -> Result<Self> {
        let canonical_id = canonical_session_id(session_id).to_string();
        let path = resolve_existing_session_path(session_id)?;
        let file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open session file: {}", path.display()))?;
        Ok(Self {
            session_id: canonical_id,
            path,
            writer: BufWriter::new(file),
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a single event as one JSONL line + flush.
    pub fn append(&mut self, event: &StorageEvent) -> Result<()> {
        serde_json::to_writer(&mut self.writer, event).context("failed to serialize StorageEvent")?;
        writeln!(self.writer).context("failed to write newline")?;
        self.writer.flush().context("failed to flush event log")?;
        Ok(())
    }

    /// Load all events from a session file.
    /// Skips blank lines. Returns error on parse failure.
    pub fn load(session_id: &str) -> Result<Vec<StorageEvent>> {
        let path = resolve_existing_session_path(session_id)?;
        Self::load_from_path(&path)
    }

    /// Load all events from a specific path.
    /// Skips blank lines. Returns error on parse failure.
    pub fn load_from_path(path: &Path) -> Result<Vec<StorageEvent>> {
        let file = File::open(&path)
            .with_context(|| format!("failed to open session file: {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line.context("failed to read line from session file")?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = serde_json::from_str::<StorageEvent>(trimmed).with_context(|| {
                format!("failed to parse event at {}:{}: {}", path.display(), i + 1, trimmed)
            })?;
            events.push(event);
        }
        Ok(events)
    }

    /// List all session ids found in the sessions directory, sorted alphabetically.
    pub fn list_sessions() -> Result<Vec<String>> {
        let dir = sessions_dir()?;
        Self::list_sessions_from_path(&dir)
    }

    pub fn list_sessions_with_meta() -> Result<Vec<SessionMeta>> {
        let dir = sessions_dir()?;
        Self::list_sessions_with_meta_from_path(&dir)
    }

    pub fn delete_session(session_id: &str) -> Result<()> {
        let dir = sessions_dir()?;
        Self::delete_session_from_path(&dir, session_id)
    }

    pub fn delete_sessions_by_working_dir(working_dir: &str) -> Result<DeleteProjectResult> {
        let dir = sessions_dir()?;
        Self::delete_sessions_by_working_dir_from_path(&dir, working_dir)
    }

    /// List session ids from a specific directory (for testing).
    fn list_sessions_from_path(dir: &Path) -> Result<Vec<String>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(dir).context("failed to read sessions directory")? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name
                .strip_prefix("session-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            {
                ids.push(id.to_string());
            }
        }
        ids.sort();
        Ok(ids)
    }

    fn list_sessions_with_meta_from_path(dir: &Path) -> Result<Vec<SessionMeta>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut metas = Vec::new();
        for entry in fs::read_dir(dir).context("failed to read sessions directory")? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(id) = name
                .strip_prefix("session-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            else {
                continue;
            };

            let canonical_id = canonical_session_id(id).to_string();
            let path = entry.path();
            let (created_at, working_dir, title) = Self::read_session_head_meta(&path)?;
            let updated_at = Self::read_last_timestamp(&path).unwrap_or(created_at);
            metas.push(SessionMeta {
                session_id: canonical_id,
                working_dir: working_dir.clone(),
                display_name: session_display_name(&working_dir),
                title,
                created_at,
                updated_at,
            });
        }

        metas.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.created_at.cmp(&a.created_at))
                .then_with(|| b.session_id.cmp(&a.session_id))
        });

        Ok(metas)
    }

    fn delete_session_from_path(dir: &Path, session_id: &str) -> Result<()> {
        let canonical_id = canonical_session_id(session_id);
        let canonical = dir.join(format!("session-{canonical_id}.jsonl"));
        let legacy = dir.join(format!("session-{session_id}.jsonl"));
        let target = if canonical.exists() {
            canonical
        } else if legacy != canonical && legacy.exists() {
            legacy
        } else {
            return Err(anyhow!("session file not found: {}", canonical.display()));
        };

        fs::remove_file(&target)
            .with_context(|| format!("failed to delete session file: {}", target.display()))?;
        Ok(())
    }

    fn delete_sessions_by_working_dir_from_path(
        dir: &Path,
        working_dir: &str,
    ) -> Result<DeleteProjectResult> {
        let metas = Self::list_sessions_with_meta_from_path(dir)?;
        let mut success_count = 0usize;
        let mut failed_session_ids = Vec::new();

        for meta in metas.into_iter().filter(|m| m.working_dir == working_dir) {
            match Self::delete_session_from_path(dir, &meta.session_id) {
                Ok(_) => success_count += 1,
                Err(_) => failed_session_ids.push(meta.session_id),
            }
        }

        Ok(DeleteProjectResult {
            success_count,
            failed_session_ids,
        })
    }

    fn read_session_head_meta(path: &Path) -> Result<(DateTime<Utc>, String, String)> {
        let file = File::open(path)
            .with_context(|| format!("failed to open session file: {}", path.display()))?;
        let reader = BufReader::new(file);

        let mut created_at = None;
        let mut working_dir = None;
        let mut title = None;

        for (i, line) in reader.lines().enumerate() {
            let line = line.context("failed to read line from session file")?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let event = serde_json::from_str::<StorageEvent>(trimmed).with_context(|| {
                format!(
                    "failed to parse head event at {}:{}: {}",
                    path.display(),
                    i + 1,
                    trimmed
                )
            })?;

            match event {
                StorageEvent::SessionStart {
                    timestamp,
                    working_dir: wd,
                    ..
                } => {
                    if created_at.is_none() {
                        created_at = Some(timestamp);
                        working_dir = Some(wd);
                    }
                }
                StorageEvent::UserMessage { content, .. } if title.is_none() => {
                    title = Some(title_from_user_message(&content));
                }
                _ => {}
            }

            if created_at.is_some() && title.is_some() {
                break;
            }
        }

        let created_at = created_at.ok_or_else(|| {
            anyhow!("session file missing sessionStart: {}", path.display())
        })?;
        let working_dir = working_dir.unwrap_or_default();
        let title = title.unwrap_or_else(|| "新会话".to_string());
        Ok((created_at, working_dir, title))
    }

    fn read_last_timestamp(path: &Path) -> Result<DateTime<Utc>> {
        let file = File::open(path)
            .with_context(|| format!("failed to open session file: {}", path.display()))?;
        let mut reader = BufReader::new(file);
        let len = reader
            .get_ref()
            .metadata()
            .with_context(|| format!("failed to stat session file: {}", path.display()))?
            .len();

        if len == 0 {
            return Err(anyhow!("empty session file: {}", path.display()));
        }

        let mut window: u64 = 4096;
        loop {
            let start = len.saturating_sub(window);
            reader.seek(SeekFrom::Start(start))?;

            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)?;

            let slice = if start > 0 {
                if let Some(pos) = bytes.iter().position(|b| *b == b'\n') {
                    &bytes[pos + 1..]
                } else {
                    if start == 0 {
                        bytes.as_slice()
                    } else {
                        if window >= len {
                            bytes.as_slice()
                        } else {
                            window = (window * 2).min(len);
                            continue;
                        }
                    }
                }
            } else {
                bytes.as_slice()
            };

            let text = String::from_utf8_lossy(slice);
            for line in text.lines().rev() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let event = serde_json::from_str::<StorageEvent>(trimmed).with_context(|| {
                    format!("failed to parse tail event at {}: {}", path.display(), trimmed)
                })?;
                if let Some(timestamp) = timestamp_of_event(&event) {
                    return Ok(timestamp);
                }
            }

            if start == 0 || window >= len {
                break;
            }
            window = (window * 2).min(len);
        }

        Err(anyhow!(
            "unable to resolve tail timestamp from session file: {}",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn make_test_log(dir: &std::path::Path) -> EventLog {
        let session_id = "test-session-001";
        let path = dir.join(format!("session-{session_id}.jsonl"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        EventLog {
            session_id: session_id.to_string(),
            path: path.clone(),
            writer: BufWriter::new(file),
        }
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
            content: "hello".into(),
            timestamp: Utc::now(),
        };

        log.append(&e1).unwrap();
        log.append(&e2).unwrap();

        let loaded = EventLog::load_from_path(log.path()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(matches!(&loaded[0], StorageEvent::SessionStart { session_id, .. } if session_id == "test-session-001"));
        assert!(matches!(&loaded[1], StorageEvent::UserMessage { content, .. } if content == "hello"));
    }

    #[test]
    fn load_errors_on_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("session-bad.jsonl");
        {
            let mut f = File::create(&path).unwrap();
            writeln!(f, r#"{{"type":"userMessage","content":"ok","timestamp":"2026-01-01T00:00:00Z"}}"#).unwrap();
            writeln!(f, "THIS IS NOT JSON").unwrap();
        }
        let result = EventLog::load_from_path(&path);
        assert!(result.is_err());
    }

    #[test]
    fn generate_session_id_format() {
        let id = generate_session_id();
        // Should match pattern like 2026-03-08T12-30-01-a3f2b1c0
        assert!(id.len() > 20);
        assert!(id.contains('T'));
        // Last segment after the datetime should be 8 hex chars
        let parts: Vec<&str> = id.rsplitn(2, '-').collect();
        assert_eq!(parts[0].len(), 8);
    }

    #[test]
    fn list_sessions_returns_sorted_ids() {
        let tmp = tempfile::tempdir().unwrap();

        // Create session files with specific IDs
        let ids = ["2026-03-01T10-00-00-aaaaaaaa", "2026-03-02T12-30-00-bbbbbbbb", "2026-03-01T09-00-00-cccccccc"];
        for id in &ids {
            let path = tmp.path().join(format!("session-{id}.jsonl"));
            let mut f = File::create(&path).unwrap();
            writeln!(f, r#"{{"type":"sessionStart","sessionId":"{id}","timestamp":"2026-01-01T00:00:00Z","workingDir":"/tmp"}}"#).unwrap();
        }

        // Create a non-session file that should be ignored
        File::create(tmp.path().join("other-file.txt")).unwrap();

        // Create a file with wrong prefix that should be ignored
        File::create(tmp.path().join("not-session-123.jsonl")).unwrap();

        // Create a directory that should be ignored
        fs::create_dir(tmp.path().join("session-dir-inside.jsonl")).unwrap();

        // Call the real function
        let found = EventLog::list_sessions_from_path(tmp.path()).unwrap();

        // Should return only the session IDs, sorted alphabetically
        assert_eq!(found.len(), 3);
        assert_eq!(found[0], "2026-03-01T09-00-00-cccccccc");
        assert_eq!(found[1], "2026-03-01T10-00-00-aaaaaaaa");
        assert_eq!(found[2], "2026-03-02T12-30-00-bbbbbbbb");
    }

    #[test]
    fn session_path_normalizes_prefixed_ids() {
        let path = session_path("session-2026-03-08T10-00-00-aaaaaaaa").unwrap();
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
                    content: "session-a-title".to_string(),
                    timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T10:01:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                },
                StorageEvent::TurnDone {
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
                    content: "session-b-title".to_string(),
                    timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T11:01:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                },
                StorageEvent::TurnDone {
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
                    timestamp: chrono::DateTime::parse_from_rfc3339("2026-03-08T13:05:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                },
            ];
            for event in events {
                writeln!(file, "{}", serde_json::to_string(&event).unwrap()).unwrap();
            }
        }

        let result = EventLog::delete_sessions_by_working_dir_from_path(tmp.path(), working_dir).unwrap();
        assert_eq!(result.success_count, 1);
        assert_eq!(
            result.failed_session_ids,
            vec!["2026-03-08T13-00-01-bbbbbbbb".to_string()]
        );
        assert!(!path_ok.exists());
    }
}
