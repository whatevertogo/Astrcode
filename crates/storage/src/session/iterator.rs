use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use astrcode_core::{StoredEvent, StoredEventLine};

use crate::Result;

/// 逐行流式读取 JSONL 会话事件。
pub struct EventLogIterator {
    lines: std::io::Lines<BufReader<File>>,
    line_number: u64,
    path: PathBuf,
}

impl EventLogIterator {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            crate::AstrError::io(
                format!("failed to open session file: {}", path.display()),
                e,
            )
        })?;
        Ok(Self {
            lines: BufReader::new(file).lines(),
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
                Ok(line) => line,
                Err(error) => {
                    return Some(Err(crate::AstrError::io(
                        "failed to read line from session file",
                        error,
                    )));
                }
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
                    return Some(Err(crate::AstrError::parse(
                        format!(
                            "failed to parse event at {}:{}: {}",
                            self.path.display(),
                            self.line_number,
                            trimmed
                        ),
                        error,
                    )));
                }
            };
            return Some(Ok(event.into_stored(self.line_number)));
        }
    }
}
