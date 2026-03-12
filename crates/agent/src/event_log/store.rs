use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::events::{StorageEvent, StoredEvent, StoredEventLine};

use super::{
    paths::{canonical_session_id, resolve_existing_session_path},
    session_path, validated_session_id, EventLog,
};

impl EventLog {
    #[cfg(test)]
    pub fn create_at_path(session_id: &str, path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory: {}", parent.display()))?;
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
            next_storage_seq: 1,
        })
    }

    pub fn create(session_id: &str) -> Result<Self> {
        let canonical_id = validated_session_id(session_id)?;
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
            session_id: canonical_id,
            path,
            writer: BufWriter::new(file),
            next_storage_seq: 1,
        })
    }

    pub fn open(session_id: &str) -> Result<Self> {
        let canonical_id = canonical_session_id(session_id).to_string();
        let path = resolve_existing_session_path(session_id)?;
        let next_storage_seq = Self::last_storage_seq_from_path(&path)?.saturating_add(1);
        let file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open session file: {}", path.display()))?;
        Ok(Self {
            session_id: canonical_id,
            path,
            writer: BufWriter::new(file),
            next_storage_seq,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&mut self, event: &StorageEvent) -> Result<StoredEvent> {
        let stored = StoredEvent {
            storage_seq: self.next_storage_seq,
            event: event.clone(),
        };

        serde_json::to_writer(&mut self.writer, &stored)
            .context("failed to serialize StoredEvent")?;
        writeln!(self.writer).context("failed to write newline")?;
        self.writer.flush().context("failed to flush event log")?;
        self.writer
            .get_ref()
            .sync_all()
            .context("failed to sync event log")?;
        self.next_storage_seq = self.next_storage_seq.saturating_add(1);
        Ok(stored)
    }

    pub fn load(session_id: &str) -> Result<Vec<StoredEvent>> {
        let path = resolve_existing_session_path(session_id)?;
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Result<Vec<StoredEvent>> {
        let file = File::open(path)
            .with_context(|| format!("failed to open session file: {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line.context("failed to read line from session file")?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = serde_json::from_str::<StoredEventLine>(trimmed).with_context(|| {
                format!(
                    "failed to parse event at {}:{}: {}",
                    path.display(),
                    i + 1,
                    trimmed
                )
            })?;
            events.push(event.into_stored((i + 1) as u64));
        }
        Ok(events)
    }

    pub fn last_storage_seq_from_path(path: &Path) -> Result<u64> {
        Ok(Self::load_from_path(path)?
            .last()
            .map(|event| event.storage_seq)
            .unwrap_or(0))
    }
}
