//! # 事件存储实现
//!
//! 实现 `EventLog` 的文件操作：创建、打开、追加、加载。

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

use astrcode_core::store::EventLogWriter;
use astrcode_core::{StorageEvent, StoredEvent};

use crate::Result;

use super::iterator::EventLogIterator;
use super::paths::{resolve_existing_session_path, session_path};

/// 文件系统 JSONL 事件日志 writer。
pub struct EventLog {
    path: PathBuf,
    writer: BufWriter<File>,
    next_storage_seq: u64,
}

impl Drop for EventLog {
    fn drop(&mut self) {
        if let Err(error) = self.writer.flush() {
            log::warn!(
                "failed to flush event log '{}' on drop: {}",
                self.path.display(),
                error
            );
            return;
        }

        if let Err(error) = self.writer.get_ref().sync_all() {
            log::warn!(
                "failed to sync event log '{}' on drop: {}",
                self.path.display(),
                error
            );
        }
    }
}

impl EventLog {
    /// 仅用于测试：在指定路径创建事件日志
    #[cfg(test)]
    pub fn create_at_path(_session_id: &str, path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                crate::AstrError::io(
                    format!("failed to create directory: {}", parent.display()),
                    e,
                )
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                crate::AstrError::io(
                    format!("failed to create session file: {}", path.display()),
                    e,
                )
            })?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            next_storage_seq: 1,
        })
    }

    /// 创建新的事件日志
    ///
    /// - `session_id` 必须符合格式要求
    /// - `working_dir` 必须可映射到确定的项目分桶目录
    /// - 文件必须不存在（`create_new` 保证）
    pub fn create(session_id: &str, working_dir: &Path) -> Result<Self> {
        let path = session_path(session_id, working_dir)?;
        if let Some(parent) = path.parent() {
            // 每个 session 单独目录，后续才能安全地给该 session 增加附件或索引文件。
            fs::create_dir_all(parent).map_err(|e| {
                crate::AstrError::io(
                    format!("failed to create sessions directory: {}", parent.display()),
                    e,
                )
            })?;
        }
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|e| {
                crate::AstrError::io(
                    format!("failed to create session file: {}", path.display()),
                    e,
                )
            })?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            next_storage_seq: 1,
        })
    }

    /// 打开现有的事件日志
    pub fn open(session_id: &str) -> Result<Self> {
        let path = resolve_existing_session_path(session_id)?;
        let next_storage_seq = Self::last_storage_seq_from_path(&path)?.saturating_add(1);
        let file = OpenOptions::new().append(true).open(&path).map_err(|e| {
            crate::AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            next_storage_seq,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append_stored(&mut self, event: &StorageEvent) -> Result<StoredEvent> {
        let stored = StoredEvent {
            storage_seq: self.next_storage_seq,
            event: event.clone(),
        };

        serde_json::to_writer(&mut self.writer, &stored)
            .map_err(|e| crate::AstrError::parse("failed to serialize StoredEvent", e))?;
        writeln!(self.writer).map_err(|e| crate::AstrError::io("failed to write newline", e))?;
        self.writer
            .flush()
            .map_err(|e| crate::AstrError::io("failed to flush event log", e))?;
        self.writer
            .get_ref()
            .sync_all()
            .map_err(|e| crate::AstrError::io("failed to sync event log", e))?;
        self.next_storage_seq = self.next_storage_seq.saturating_add(1);
        Ok(stored)
    }

    pub fn replay_to<F>(path: &Path, mut callback: F) -> Result<()>
    where
        F: FnMut(&StoredEvent) -> Result<()>,
    {
        for event_result in EventLogIterator::from_path(path)? {
            callback(&event_result?)?;
        }
        Ok(())
    }

    pub fn last_storage_seq_from_path(path: &Path) -> Result<u64> {
        let file_size = std::fs::metadata(path)
            .map_err(|e| crate::AstrError::io("failed to read file metadata", e))?
            .len();

        if file_size == 0 {
            return Ok(0);
        }

        const TAIL_THRESHOLD: u64 = 64 * 1024;
        if file_size <= TAIL_THRESHOLD {
            let mut last_seq: Option<u64> = None;
            for event_result in EventLogIterator::from_path(path)? {
                let event = event_result?;
                last_seq = Some(event.storage_seq);
            }
            return Ok(last_seq.unwrap_or(0));
        }

        let offset = file_size - TAIL_THRESHOLD;
        let mut file = File::open(path).map_err(|e| {
            crate::AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        let started_mid_line = if offset == 0 {
            false
        } else {
            file.seek(std::io::SeekFrom::Start(offset - 1))
                .map_err(|e| crate::AstrError::io("failed to seek in session file", e))?;
            let mut previous = [0u8; 1];
            file.read_exact(&mut previous)
                .map_err(|e| crate::AstrError::io("failed to inspect session file tail", e))?;
            previous[0] != b'\n'
        };
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| crate::AstrError::io("failed to seek in session file", e))?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| crate::AstrError::io("failed to read session file tail", e))?;

        if started_mid_line {
            let Some((_, remaining)) = content.split_once('\n') else {
                let mut last_seq: Option<u64> = None;
                for event_result in EventLogIterator::from_path(path)? {
                    let event = event_result?;
                    last_seq = Some(event.storage_seq);
                }
                return Ok(last_seq.unwrap_or(0));
            };
            content = remaining.to_string();
        }

        for line in content.lines().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(seq) = serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .and_then(|v| v.get("storage_seq").and_then(|s| s.as_u64()))
            {
                return Ok(seq);
            }
        }

        let mut last_seq: Option<u64> = None;
        for event_result in EventLogIterator::from_path(path)? {
            let event = event_result?;
            last_seq = Some(event.storage_seq);
        }
        Ok(last_seq.unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrcode_core::StorageEvent;
    use chrono::Utc;

    #[test]
    fn last_storage_seq_tail_scan_skips_partial_first_line() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let path = temp_dir.path().join("session-test-session.jsonl");
        let mut log = EventLog::create_at_path("test-session", path.clone()).expect("event log");

        for index in 0..3 {
            log.append_stored(&StorageEvent::AssistantFinal {
                turn_id: Some(format!("turn-{index}")),
                content: "x".repeat(40_000),
                reasoning_content: None,
                reasoning_signature: None,
                timestamp: Some(Utc::now()),
            })
            .expect("append should succeed");
        }

        assert_eq!(
            EventLog::last_storage_seq_from_path(&path).expect("tail scan should succeed"),
            3
        );
    }
}

impl EventLogWriter for EventLog {
    fn append(&mut self, event: &StorageEvent) -> Result<StoredEvent> {
        self.append_stored(event)
    }
}
