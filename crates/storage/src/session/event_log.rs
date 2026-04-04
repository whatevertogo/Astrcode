//! # 事件日志实现
//!
//! 提供 `EventLog` 结构体，负责 JSONL 会话文件的创建、打开、追加写入与回放。
//!
//! ## 设计要点
//!
//! - **Append-only 模型**：每个事件以 `StoredEvent { storage_seq, event }` 格式追加写入，
//!   `storage_seq` 单调递增且由 writer 独占分配，保证事件全局有序。
//! - **同步刷盘**：每次 `append_stored` 后执行 `flush` + `sync_all`，确保数据持久化到磁盘，
//!   避免进程崩溃导致事件丢失。
//! - **Drop 安全**：`Drop` 实现中再次 flush 和 sync，防止遗漏未刷盘的数据。
//! - **尾部扫描优化**：`last_storage_seq_from_path` 对大文件只读取尾部 64KB， 避免全量加载整个
//!   JSONL 文件。

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Seek, Write},
    path::{Path, PathBuf},
};

use astrcode_core::{StorageEvent, StoredEvent, store::EventLogWriter};

use super::{
    iterator::EventLogIterator,
    paths::{resolve_existing_session_path, session_path},
};
use crate::Result;

/// 文件系统 JSONL 事件日志 writer。
///
/// 封装了对会话 JSONL 文件的写入操作，维护 `next_storage_seq` 以保证
/// 每个事件的 `storage_seq` 单调递增。每次追加写入后自动 flush 并 sync 到磁盘。
///
/// ## 生命周期
///
/// 通过 `Drop` 实现确保未刷盘的数据在对象销毁时写入磁盘。
pub struct EventLog {
    /// 会话 JSONL 文件的完整路径。
    path: PathBuf,
    /// 缓冲写入器，减少系统调用次数。
    writer: BufWriter<File>,
    /// 下一个事件的 storage_seq，从 1 开始单调递增。
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
    /// 仅用于测试：在指定路径创建事件日志。
    ///
    /// 绕过正常路径解析逻辑，直接在给定路径创建文件，
    /// 以便测试可以精确控制文件位置。
    #[cfg(test)]
    pub fn create_at_path(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                crate::io_error(
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
                crate::io_error(
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

    /// 创建新的事件日志。
    ///
    /// 根据 `session_id` 和 `working_dir` 解析出完整的 JSONL 文件路径，
    /// 使用 `create_new(true)` 确保文件不存在，避免覆盖已有会话。
    ///
    /// ## 参数约束
    ///
    /// - `session_id` 必须符合格式要求（仅含字母数字、`-`、`_`、`T`）
    /// - `working_dir` 必须可映射到确定的项目分桶目录
    /// - 文件必须不存在（`create_new` 保证）
    pub fn create(session_id: &str, working_dir: &Path) -> Result<Self> {
        let path = session_path(session_id, working_dir)?;
        if let Some(parent) = path.parent() {
            // 每个 session 单独目录，后续才能安全地给该 session 增加附件或索引文件。
            fs::create_dir_all(parent).map_err(|e| {
                crate::io_error(
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
                crate::io_error(
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

    /// 打开现有的事件日志。
    ///
    /// 通过扫描所有项目的 sessions 目录查找匹配的 session 文件，
    /// 并从文件尾部推断下一个 `storage_seq`，确保续写时序列号连续。
    pub fn open(session_id: &str) -> Result<Self> {
        let path = resolve_existing_session_path(session_id)?;
        let next_storage_seq = Self::last_storage_seq_from_path(&path)?.saturating_add(1);
        let file = OpenOptions::new().append(true).open(&path).map_err(|e| {
            crate::io_error(
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

    /// 返回事件日志文件的完整路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加一个存储事件到 JSONL 文件。
    ///
    /// 将 `StorageEvent` 包装为 `StoredEvent`（附带 `storage_seq`），
    /// 序列化为 JSON 行写入文件，然后立即 flush 并 sync 到磁盘。
    /// 返回包含已分配 `storage_seq` 的 `StoredEvent`。
    pub fn append_stored(&mut self, event: &StorageEvent) -> Result<StoredEvent> {
        let stored = StoredEvent {
            storage_seq: self.next_storage_seq,
            event: event.clone(),
        };

        serde_json::to_writer(&mut self.writer, &stored)
            .map_err(|e| crate::parse_error("failed to serialize StoredEvent", e))?;
        writeln!(self.writer).map_err(|e| crate::io_error("failed to write newline", e))?;
        self.writer
            .flush()
            .map_err(|e| crate::io_error("failed to flush event log", e))?;
        self.writer
            .get_ref()
            .sync_all()
            .map_err(|e| crate::io_error("failed to sync event log", e))?;
        self.next_storage_seq = self.next_storage_seq.saturating_add(1);
        Ok(stored)
    }

    /// 回放指定路径的会话文件中的所有事件。
    ///
    /// 通过 [`EventLogIterator`] 逐行读取并调用回调函数，用于
    /// 会话重建或事件流订阅场景。
    pub fn replay_to<F>(path: &Path, mut callback: F) -> Result<()>
    where
        F: FnMut(&StoredEvent) -> Result<()>,
    {
        for event_result in EventLogIterator::from_path(path)? {
            callback(&event_result?)?;
        }
        Ok(())
    }

    /// 从会话文件尾部扫描最后一个 `storage_seq`。
    ///
    /// 对于小文件（≤64KB）全量扫描；对于大文件只读取尾部 64KB，
    /// 从后往前查找第一个包含 `storage_seq` 的 JSON 行。
    /// 如果尾部扫描未命中（例如截断点恰好在关键行上），则回退到全量扫描。
    ///
    /// 此方法用于 `open()` 时确定下一个 `storage_seq`，保证续写时序列号连续。
    pub fn last_storage_seq_from_path(path: &Path) -> Result<u64> {
        let file_size = std::fs::metadata(path)
            .map_err(|e| Self::enhance_metadata_error(path, e))?
            .len();

        if file_size == 0 {
            return Ok(0);
        }

        const TAIL_THRESHOLD: u64 = 64 * 1024;
        if file_size <= TAIL_THRESHOLD {
            return Self::scan_full_file_for_last_seq(path);
        }

        let offset = file_size - TAIL_THRESHOLD;
        let mut file = File::open(path).map_err(|e| Self::enhance_open_error(path, e))?;
        let started_mid_line = if offset == 0 {
            false
        } else {
            file.seek(std::io::SeekFrom::Start(offset - 1))
                .map_err(|e| crate::io_error("failed to seek in session file", e))?;
            let mut previous = [0u8; 1];
            file.read_exact(&mut previous)
                .map_err(|e| crate::io_error("failed to inspect session file tail", e))?;
            previous[0] != b'\n'
        };
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| crate::io_error("failed to seek in session file", e))?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| Self::enhance_read_error(path, e))?;

        if started_mid_line {
            let Some((_, remaining)) = content.split_once('\n') else {
                return Self::scan_full_file_for_last_seq(path);
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

        Self::scan_full_file_for_last_seq(path)
    }

    /// 全量扫描文件，返回最后一个事件的 storage_seq。
    fn scan_full_file_for_last_seq(path: &Path) -> Result<u64> {
        let mut last_seq: Option<u64> = None;
        for event_result in EventLogIterator::from_path(path)? {
            let event = event_result?;
            last_seq = Some(event.storage_seq);
        }
        Ok(last_seq.unwrap_or(0))
    }

    /// 增强 metadata 读取错误的提示信息。
    ///
    /// 根据错误类型提供更具体的诊断信息，帮助用户定位问题。
    fn enhance_metadata_error(path: &Path, e: std::io::Error) -> crate::StoreError {
        use std::io::ErrorKind;

        let hint = match e.kind() {
            ErrorKind::PermissionDenied => format!(
                "permission denied: cannot access session file '{}'. Check if the file is owned \
                 by another user or has restrictive permissions.",
                path.display()
            ),
            ErrorKind::NotFound => format!(
                "session file '{}' not found. The session may have been deleted or moved.",
                path.display()
            ),
            _ => format!(
                "failed to read metadata for session file '{}'",
                path.display()
            ),
        };
        crate::io_error(hint, e)
    }

    /// 增强文件打开错误的提示信息。
    ///
    /// 针对常见的打开失败原因（权限、锁定等）提供具体诊断。
    fn enhance_open_error(path: &Path, e: std::io::Error) -> crate::StoreError {
        use std::io::ErrorKind;

        let hint = match e.kind() {
            ErrorKind::PermissionDenied => format!(
                "permission denied: cannot open session file '{}'. Check file permissions or if \
                 another process has locked it.",
                path.display()
            ),
            ErrorKind::NotFound => format!(
                "session file '{}' not found. The session may have been deleted.",
                path.display()
            ),
            _ => format!("failed to open session file '{}'", path.display()),
        };
        crate::io_error(hint, e)
    }

    /// 增强文件读取错误的提示信息。
    ///
    /// 特别处理非 UTF-8 数据的情况，这是会话文件损坏的常见原因。
    fn enhance_read_error(path: &Path, e: std::io::Error) -> crate::StoreError {
        use std::io::ErrorKind;

        let hint = match e.kind() {
            ErrorKind::InvalidData => format!(
                "session file '{}' contains invalid UTF-8 data. The file may be corrupted or \
                 truncated. Consider deleting this session to recover.",
                path.display()
            ),
            ErrorKind::PermissionDenied => format!(
                "permission denied while reading session file '{}'.",
                path.display()
            ),
            ErrorKind::UnexpectedEof => format!(
                "unexpected end of session file '{}'. The file may be truncated or still being \
                 written.",
                path.display()
            ),
            _ => format!(
                "failed to read session file '{}' (I/O error: {})",
                path.display(),
                e
            ),
        };
        crate::io_error(hint, e)
    }
}

impl EventLogWriter for EventLog {
    fn append(&mut self, event: &StorageEvent) -> Result<StoredEvent> {
        self.append_stored(event)
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::StorageEvent;
    use chrono::Utc;

    use super::*;

    #[test]
    fn last_storage_seq_tail_scan_skips_partial_first_line() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let path = temp_dir.path().join("session-test-session.jsonl");
        let mut log = EventLog::create_at_path(path.clone()).expect("event log");

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
