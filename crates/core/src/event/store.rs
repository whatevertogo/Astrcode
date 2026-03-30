//! # 事件存储实现
//!
//! 实现 `EventLog` 的文件操作：创建、打开、追加、加载。

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

impl EventLog {
    /// 仅用于测试：在指定路径创建事件日志
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

    /// 创建新的事件日志
    ///
    /// ## 验证
    ///
    /// - `session_id` 必须符合格式要求
    /// - 文件必须不存在（`create_new` 保证）
    ///
    /// ## 失败情况
    ///
    /// - 会话 ID 格式无效
    /// - 文件已存在
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

    /// 打开现有的事件日志
    ///
    /// 自动扫描文件以确定下一个 `storage_seq`。
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

    /// 获取会话 ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 获取日志文件路径
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一个事件到日志
    ///
    /// ## 持久化保证
    ///
    /// 1. 序列化为 JSON
    /// 2. 写入换行符
    /// 3. `flush()` - 确保数据从用户态缓冲区写入内核
    /// 4. `sync_all()` - 确保数据从内核页缓存写入磁盘
    ///
    /// 这个顺序确保即使进程崩溃，已追加的事件也不会丢失。
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

    /// 加载会话的所有事件
    pub fn load(session_id: &str) -> Result<Vec<StoredEvent>> {
        let path = resolve_existing_session_path(session_id)?;
        Self::load_from_path(&path)
    }

    /// 从文件路径加载所有事件
    ///
    /// ## 容错处理
    ///
    /// - 空行被跳过（允许尾随换行）
    /// - 解析错误会返回包含行号和内容的有用错误信息
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

    /// 获取文件中最后一个事件的 storage_seq
    ///
    /// 用于打开现有日志时确定下一个序号。
    pub fn last_storage_seq_from_path(path: &Path) -> Result<u64> {
        Ok(Self::load_from_path(path)?
            .last()
            .map(|event| event.storage_seq)
            .unwrap_or(0))
    }
}
