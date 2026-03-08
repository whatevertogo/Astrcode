use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use uuid::Uuid;

use crate::events::StorageEvent;

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

fn session_path(session_id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("session-{session_id}.jsonl")))
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
        let path = session_path(session_id)?;
        if !path.exists() {
            return Err(anyhow!(
                "session file not found: {}",
                path.display()
            ));
        }
        let file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open session file: {}", path.display()))?;
        Ok(Self {
            session_id: session_id.to_string(),
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
        let path = session_path(session_id)?;
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
}
