//! # 事件存储实现
//!
//! 实现 `EventLog` 的文件操作：创建、打开、追加、加载。

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, Write};
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

    /// 从文件路径加载所有事件到内存中（保留兼容）
    ///
    /// # 注意
    /// 此方法会将整个 JSONL 文件加载到内存中。对于大型会话文件，
    /// 建议使用 [`iter_from_path`](Self::iter_from_path) 或 [`replay_to`](Self::replay_to) 进行流式处理。
    ///
    /// ## 容错处理
    ///
    /// - 空行被跳过（允许尾随换行）
    /// - 解析错误会返回包含行号和内容的有用错误信息
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

    /// 获取文件中最后一个事件的 storage_seq
    ///
    /// 用于打开现有日志时确定下一个序号。
    /// 小文件直接迭代；大文件仅读取尾部避免全量扫描。
    pub fn last_storage_seq_from_path(path: &Path) -> Result<u64> {
        let file_size = std::fs::metadata(path)
            .map_err(|e| crate::AstrError::io("failed to read file metadata", e))?
            .len();

        if file_size == 0 {
            return Ok(0);
        }

        // 小文件（<= 64KB）直接全量迭代，避免 seek 在 Windows 上
        // 与并发写入句柄的交互问题
        const TAIL_THRESHOLD: u64 = 64 * 1024;
        if file_size <= TAIL_THRESHOLD {
            let mut last_seq: Option<u64> = None;
            for event_result in EventLogIterator::from_path(path)? {
                let event = event_result?;
                last_seq = Some(event.storage_seq);
            }
            return Ok(last_seq.unwrap_or(0));
        }

        // 大文件：仅读取尾部 64KB，避免扫描数百 MB 的会话文件
        let offset = file_size - TAIL_THRESHOLD;

        let mut file = File::open(path).map_err(|e| {
            crate::AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(|e| crate::AstrError::io("failed to seek in session file", e))?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| crate::AstrError::io("failed to read session file tail", e))?;

        // 从后向前搜索最后一个有效事件行，找到即返回
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

        // 尾部未找到有效事件（极端情况：单行超大事件），回退到全量扫描
        let mut last_seq: Option<u64> = None;
        for event_result in EventLogIterator::from_path(path)? {
            let event = event_result?;
            last_seq = Some(event.storage_seq);
        }
        Ok(last_seq.unwrap_or(0))
    }
}
