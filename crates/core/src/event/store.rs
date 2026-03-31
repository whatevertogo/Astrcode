use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use crate::Result;

use crate::event::{StorageEvent, StoredEvent, StoredEventLine};

use super::{
    paths::{canonical_session_id, resolve_existing_session_path},
    session_path, validated_session_id, EventLog,
};

/// 迭代器，用于流式读取 JSONL 事件文件
///
/// 该迭代器逐行读取文件，不会将整个文件加载到内存中。
/// 适用于大型会话文件（数百 MB）的场景。
pub struct EventLogIterator {
    lines: std::io::Lines<BufReader<File>>,
    line_number: u64,
    path: std::path::PathBuf,
}

impl EventLogIterator {
    /// 从文件路径创建新的迭代器
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            crate::AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        let reader = BufReader::new(file);
        Ok(Self {
            lines: reader.lines(),
            line_number: 0,
            path: path.to_path_buf(),
        })
    }
}

impl Iterator for EventLogIterator {
    type Item = Result<StoredEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = match self.lines.next()? {
                Ok(l) => l,
                Err(e) => {
                    return Some(Err(crate::AstrError::io(
                        "failed to read line from session file",
                        e,
                    )));
                }
            };
            self.line_number += 1;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = match serde_json::from_str::<StoredEventLine>(trimmed) {
                Ok(e) => e,
                Err(e) => {
                    return Some(Err(crate::AstrError::parse(
                        format!(
                            "failed to parse event at {}:{}: {}",
                            self.path.display(),
                            self.line_number,
                            trimmed
                        ),
                        e,
                    )));
                }
            };
            return Some(Ok(event.into_stored(self.line_number)));
        }
    }
}

impl EventLog {
    #[cfg(test)]
    pub fn create_at_path(session_id: &str, path: PathBuf) -> Result<Self> {
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
            .map_err(|e| {
                crate::AstrError::io(
                    format!("failed to open session file: {}", path.display()),
                    e,
                )
            })?;
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

    /// 加载会话的所有事件到内存中
    ///
    /// # 注意
    /// 此方法会将整个 JSONL 文件加载到内存中。对于大型会话文件，
    /// 建议使用 [`iter_from_path`](Self::iter_from_path) 或 [`replay_to`](Self::replay_to) 进行流式处理。
    #[deprecated(
        since = "0.2.0",
        note = "Use iter_from_path or replay_to for streaming"
    )]
    pub fn load(session_id: &str) -> Result<Vec<StoredEvent>> {
        let path = resolve_existing_session_path(session_id)?;
        Self::load_from_path(&path)
    }

    /// 从路径加载会话的所有事件到内存中（保留兼容）
    ///
    /// # 注意
    /// 此方法会将整个 JSONL 文件加载到内存中。对于大型会话文件，
    /// 建议使用 [`iter_from_path`](Self::iter_from_path) 或 [`replay_to`](Self::replay_to) 进行流式处理。
    #[deprecated(
        since = "0.2.0",
        note = "Use iter_from_path or replay_to for streaming"
    )]
    pub fn load_from_path(path: &Path) -> Result<Vec<StoredEvent>> {
        let file = File::open(path).map_err(|e| {
            crate::AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line =
                line.map_err(|e| crate::AstrError::io("failed to read line from session file", e))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = serde_json::from_str::<StoredEventLine>(trimmed).map_err(|e| {
                crate::AstrError::parse(
                    format!(
                        "failed to parse event at {}:{}: {}",
                        path.display(),
                        i + 1,
                        trimmed
                    ),
                    e,
                )
            })?;
            events.push(event.into_stored((i + 1) as u64));
        }
        Ok(events)
    }

    /// 返回迭代器，支持逐行流式读取事件
    ///
    /// 该方法不会将整个文件加载到内存中，而是返回一个迭代器，
    /// 每次调用 `next()` 时读取并解析下一行。
    ///
    /// # 示例
    /// ```ignore
    /// for event_result in EventLog::iter_from_path(&path)? {
    ///     let event = event_result?;
    ///     println!("Event {}: {:?}", event.storage_seq, event.event);
    /// }
    /// ```
    pub fn iter_from_path(path: &Path) -> Result<EventLogIterator> {
        EventLogIterator::from_path(path)
    }

    /// 流式读取事件并调用回调函数
    ///
    /// 该方法逐行读取文件，对每个成功解析的事件调用 `callback`。
    /// 如果回调返回 `Err`，则立即停止读取并返回错误。
    ///
    /// # 参数
    /// * `path` - JSONL 文件路径
    /// * `callback` - 每个事件调用的回调函数，接收 `&StoredEvent` 参数
    ///
    /// # 示例
    /// ```ignore
    /// EventLog::replay_to(&path, |event| {
    ///     println!("Processing event {}", event.storage_seq);
    ///     Ok(())
    /// })?;
    /// ```
    pub fn replay_to<F>(path: &Path, mut callback: F) -> Result<()>
    where
        F: FnMut(&StoredEvent) -> Result<()>,
    {
        // 委托给 EventLogIterator，避免重复行解析逻辑
        for event_result in EventLogIterator::from_path(path)? {
            callback(&event_result?)?;
        }
        Ok(())
    }

    pub fn last_storage_seq_from_path(path: &Path) -> Result<u64> {
        // 使用迭代器只读取最后一行，避免全量加载
        let mut last_seq: Option<u64> = None;
        for event_result in EventLogIterator::from_path(path)? {
            let event = event_result?;
            last_seq = Some(event.storage_seq);
        }
        Ok(last_seq.unwrap_or(0))
    }
}
