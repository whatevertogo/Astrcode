//! # JSONL 事件迭代器
//!
//! 提供 `EventLogIterator`，逐行流式读取 JSONL 会话文件中的 `StoredEvent`。
//!
//! ## 设计要点
//!
//! - **流式读取**：使用 `BufReader::lines()` 逐行读取，不会将整个文件加载到内存，
//!   适合任意大小的会话文件。
//! - **空行跳过**：自动跳过空白行，容忍文件末尾的多余换行。
//! - **错误定位**：`line_number` 追踪物理行号（含空行），错误消息中的行号与
//!   文本编辑器中显示的行号一致，方便调试。

use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use astrcode_core::{StoredEvent, StoredEventLine};

use crate::Result;

/// 逐行流式读取 JSONL 会话事件的迭代器。
///
/// 每次 `next()` 调用读取一行 JSON，反序列化为 `StoredEventLine`，
/// 并通过 `into_stored(line_number)` 附加物理行号生成 `StoredEvent`。
/// 空行会被自动跳过，解析错误会作为 `Err` 返回而非 panic。
pub struct EventLogIterator {
    /// 底层缓冲读取器的行迭代器。
    lines: std::io::Lines<BufReader<File>>,
    /// 当前物理行号（从 1 开始，含空行），用于错误定位和事件行号标记。
    line_number: u64,
    /// 文件路径，用于错误消息中的上下文展示。
    path: PathBuf,
}

impl EventLogIterator {
    /// 从指定路径打开 JSONL 文件并创建迭代器。
    ///
    /// 文件必须存在且可读，否则返回 IO 错误。
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Self::enhance_open_error(path, e))?;
        Ok(Self {
            lines: BufReader::new(file).lines(),
            line_number: 0,
            path: path.to_path_buf(),
        })
    }

    /// 增强文件打开错误的提示信息。
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
}

impl Iterator for EventLogIterator {
    type Item = Result<StoredEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = match self.lines.next()? {
                Ok(line) => line,
                Err(error) => {
                    return Some(Err(Self::enhance_read_error(&self.path, error)));
                },
            };
            // line_number 在空行检查之前递增，因此它追踪的是文件物理行号
            // （含空行），而非逻辑事件索引。这样错误消息中的行号与文本编辑器
            // 中看到的行号一致，方便调试定位。
            self.line_number += 1;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = match serde_json::from_str::<StoredEventLine>(trimmed) {
                Ok(event) => event,
                Err(error) => {
                    return Some(Err(Self::enhance_parse_error(
                        &self.path,
                        self.line_number,
                        trimmed,
                        error,
                    )));
                },
            };
            return Some(Ok(event.into_stored(self.line_number)));
        }
    }
}

impl EventLogIterator {
    /// 增强读取行错误的提示信息。
    fn enhance_read_error(path: &Path, e: std::io::Error) -> crate::StoreError {
        use std::io::ErrorKind;

        let hint = match e.kind() {
            ErrorKind::InvalidData => format!(
                "session file '{}' contains invalid UTF-8 data. The file may be corrupted. \
                 Consider deleting this session.",
                path.display()
            ),
            ErrorKind::UnexpectedEof => format!(
                "unexpected end of session file '{}'. The file may be truncated.",
                path.display()
            ),
            _ => format!(
                "failed to read from session file '{}': {}",
                path.display(),
                e
            ),
        };
        crate::io_error(hint, e)
    }

    /// 增强解析错误的提示信息。
    fn enhance_parse_error(
        path: &Path,
        line_number: u64,
        content: &str,
        e: serde_json::Error,
    ) -> crate::StoreError {
        // 截断过长的内容，避免错误消息过长
        let preview = if content.len() > 100 {
            format!("{}...", &content[..100])
        } else {
            content.to_string()
        };
        crate::parse_error(
            format!(
                "failed to parse event at {}:{} (content: '{}'). The session file may be \
                 corrupted.",
                path.display(),
                line_number,
                preview
            ),
            e,
        )
    }
}
