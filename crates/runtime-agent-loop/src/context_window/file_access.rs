//! # 文件访问跟踪器 (File Access Tracker)
//!
//! 跟踪会话中通过工具调用访问的文件路径，用于压缩后的附件恢复。
//! 从 `StorageEvent::ToolResult` 的 `metadata` 字段中提取文件路径。
//!
//! ## 核心类型
//! - `FileAccessEntry`: 记录访问过的文件路径和触发工具名
//! - `FileAccessTracker`: 有界环形缓冲区，保留最近 N 个文件（默认 10 个）， 用于 post-compact
//!   重建附件时知道之前用过哪些文件

use std::path::PathBuf;

use astrcode_core::{StorageEvent, StorageEventPayload, StoredEvent};

/// 标记为文件访问跟踪的工具名列表，其 `metadata` 中包含 `"path"` 字段。
const FILE_TOOLS: &[&str] = &["readFile", "editFile", "writeFile"];

/// 用于 post-compact 恢复而保留的最近文件数上限。
const DEFAULT_MAX_TRACKED_FILES: usize = 10;

/// A tracked file access with the source tool name.
#[derive(Debug, Clone)]
#[allow(dead_code)] // 后续可按工具名区分 read/edit/write 的恢复优先级
pub(crate) struct FileAccessEntry {
    pub path: PathBuf,
    #[allow(dead_code)]
    pub tool_name: String,
}
/// Tracks file paths accessed during a session via tool calls.
///
/// The tracker inspects `StorageEvent::ToolResult` events and extracts
/// the `"path"` field from `metadata` for file-related tools (`readFile`,
/// `editFile`, `writeFile`).
#[derive(Debug, Clone, Default)]
pub(crate) struct FileAccessTracker {
    entries: Vec<FileAccessEntry>,
    max_entries: usize,
}
impl FileAccessTracker {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: DEFAULT_MAX_TRACKED_FILES,
        }
    }

    /// 从 durable stored events 构造 tracker。
    ///
    /// compact 重建需要同时看“当前 turn 内刚发生的工具结果”和“最近保留 tail 里
    /// 已经持久化的工具结果”，因此这里允许直接用 stored event 种子化 tracker。
    pub(crate) fn from_stored_events(events: &[StoredEvent]) -> Self {
        let mut tracker = Self::new();
        tracker.record_stored_events(events);
        tracker
    }

    /// Record a storage event if it is a file-access tool result.
    pub(crate) fn record_event(&mut self, event: &StorageEvent) {
        let StorageEventPayload::ToolResult {
            tool_name,
            metadata,
            ..
        } = &event.payload
        else {
            return;
        };
        if !FILE_TOOLS.contains(&tool_name.as_str()) {
            return;
        }
        let Some(metadata) = metadata else {
            return;
        };

        if let Some(path_str) = metadata.get("path").and_then(|v| v.as_str()) {
            let path = PathBuf::from(path_str);
            // 去重： 如果路径已被跟踪，移到末尾（最近访问）
            self.entries.retain(|entry| entry.path != path);
            self.entries.push(FileAccessEntry {
                path,
                tool_name: tool_name.clone(),
            });
            // 裁剪到最大条目数
            while self.entries.len() > self.max_entries {
                self.entries.remove(0);
            }
        }
    }

    /// 将一批持久化事件回放到 tracker。
    ///
    /// 这里复用 `record_event`，确保 live 事件和 durable 事件使用同一套提取规则。
    pub(crate) fn record_stored_events(&mut self, events: &[StoredEvent]) {
        for stored in events {
            self.record_event(&stored.event);
        }
    }

    /// Return the N most recently accessed distinct file paths.
    pub(crate) fn recent_files(&self, n: usize) -> Vec<PathBuf> {
        self.entries
            .iter()
            .rev()
            .take(n)
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Clear all tracked entries.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::AgentEventContext;
    use serde_json::json;

    use super::*;

    fn tool_result(tool_name: &str, path: &str) -> StorageEvent {
        StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::ToolResult {
                tool_call_id: "call-1".to_string(),
                tool_name: tool_name.to_string(),
                output: "file content".to_string(),
                success: true,
                error: None,
                metadata: Some(json!({"path": path})),
                duration_ms: 10,
            },
        }
    }

    fn tool_result_no_metadata(tool_name: &str) -> StorageEvent {
        StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::ToolResult {
                tool_call_id: "call-1".to_string(),
                tool_name: tool_name.to_string(),
                output: "file content".to_string(),
                success: true,
                error: None,
                metadata: None,
                duration_ms: 10,
            },
        }
    }
    #[test]
    fn records_read_file_path_from_metadata() {
        let mut tracker = FileAccessTracker::new();
        tracker.record_event(&tool_result("readFile", "/src/main.rs"));
        let recent = tracker.recent_files(5);
        assert_eq!(recent, vec![PathBuf::from("/src/main.rs")]);
    }

    #[test]
    fn records_edit_and_write_file_paths() {
        let mut tracker = FileAccessTracker::new();
        tracker.record_event(&tool_result("readFile", "/src/lib.rs"));
        tracker.record_event(&tool_result("editFile", "/src/main.rs"));
        tracker.record_event(&tool_result("writeFile", "/src/new.rs"));

        let recent = tracker.recent_files(3);
        assert_eq!(
            recent,
            vec![
                PathBuf::from("/src/new.rs"),
                PathBuf::from("/src/main.rs"),
                PathBuf::from("/src/lib.rs"),
            ]
        );
    }

    #[test]
    fn ignores_non_file_tools() {
        let mut tracker = FileAccessTracker::new();
        tracker.record_event(&tool_result("grep", "/src/main.rs"));

        assert!(tracker.recent_files(5).is_empty());
    }

    #[test]
    fn ignores_events_without_metadata() {
        let mut tracker = FileAccessTracker::new();
        tracker.record_event(&tool_result_no_metadata("readFile"));

        assert!(tracker.recent_files(5).is_empty());
    }

    #[test]
    fn ignores_non_tool_events() {
        let mut tracker = FileAccessTracker::new();
        tracker.record_event(&StorageEvent {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::UserMessage {
                content: "hello".to_string(),
                timestamp: chrono::Utc::now(),
                origin: astrcode_core::UserMessageOrigin::User,
            },
        });

        assert!(tracker.recent_files(5).is_empty());
    }

    #[test]
    fn deduplicates_paths_and_keeps_most_recent() {
        let mut tracker = FileAccessTracker::new();
        tracker.record_event(&tool_result("readFile", "/src/lib.rs"));
        tracker.record_event(&tool_result("readFile", "/src/main.rs"));
        tracker.record_event(&tool_result("readFile", "/src/lib.rs")); // re-read

        let recent = tracker.recent_files(5);
        assert_eq!(
            recent,
            vec![
                PathBuf::from("/src/lib.rs"), // most recent
                PathBuf::from("/src/main.rs"),
            ]
        );
    }

    #[test]
    fn trims_to_max_entries() {
        let mut tracker = FileAccessTracker::new();
        for i in 0..15 {
            tracker.record_event(&tool_result("readFile", &format!("/src/file{i}.rs")));
        }

        assert_eq!(tracker.entries.len(), DEFAULT_MAX_TRACKED_FILES);
        // Most recent should be last
        assert_eq!(
            tracker.entries.last().unwrap().path,
            PathBuf::from("/src/file14.rs")
        );
    }

    #[test]
    fn recent_files_returns_n_most_recent() {
        let mut tracker = FileAccessTracker::new();
        tracker.record_event(&tool_result("readFile", "/a.rs"));
        tracker.record_event(&tool_result("readFile", "/b.rs"));
        tracker.record_event(&tool_result("readFile", "/c.rs"));

        let recent = tracker.recent_files(2);
        assert_eq!(recent, vec![PathBuf::from("/c.rs"), PathBuf::from("/b.rs")]);
    }

    #[test]
    fn from_stored_events_replays_durable_tool_results() {
        let stored = vec![
            StoredEvent {
                storage_seq: 1,
                event: tool_result("readFile", "/src/lib.rs"),
            },
            StoredEvent {
                storage_seq: 2,
                event: tool_result("editFile", "/src/main.rs"),
            },
        ];

        let tracker = FileAccessTracker::from_stored_events(&stored);

        assert_eq!(
            tracker.recent_files(5),
            vec![PathBuf::from("/src/main.rs"), PathBuf::from("/src/lib.rs")]
        );
    }
}
