//! # ListDir 工具
//!
//! 实现 `listDir` 工具，用于浅层列出目录内容。
//!
//! ## 设计要点
//!
//! - 仅返回一层目录/文件条目，不递归
//! - 每个条目返回 `name`、`isDir`、`isFile`、`size`、`modified`、`extension`
//! - 默认最多 200 条，超出标记 `truncated`
//! - 未指定路径时使用上下文工作目录
//! - 支持排序：按名称（默认）或按修改时间（最新优先）

use std::{fs, path::PathBuf, time::Instant};

use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::tools::fs_common::{check_cancel, resolve_path};

/// ListDir 工具实现。
///
/// 列出指定目录的直接子条目（不递归），返回名称和类型信息。
#[derive(Default)]
pub struct ListDirTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListDirArgs {
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    max_entries: Option<usize>,
    /// 排序方式：name（默认）或 modified
    #[serde(default)]
    sort_by: Option<SortBy>,
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
enum SortBy {
    #[default]
    Name,
    Modified,
}

/// 将 Unix 时间戳转换为 ISO 8601 格式字符串。
///
/// 不依赖 chrono，使用简单的时间计算。
fn time_at(unix_secs: u64) -> String {
    // 简化实现：返回 Unix 时间戳 ISO 8601 近似格式
    // 由于没有 chrono，我们使用一个简单的格式
    let days = unix_secs / 86400;
    let years = 1970 + days / 365;
    let remaining_days = days % 365;
    let months = remaining_days / 30 + 1;
    let day = remaining_days % 30 + 1;
    let hours = (unix_secs % 86400) / 3600;
    let minutes = (unix_secs % 3600) / 60;
    let seconds = unix_secs % 60;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        years, months, day, hours, minutes, seconds
    )
}

/// 目录条目信息。
#[derive(Debug, Clone)]
struct DirEntry {
    name: String,
    is_dir: bool,
    is_file: bool,
    size: u64,
    modified: Option<std::time::SystemTime>,
    extension: Option<String>,
}

#[async_trait]
impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "listDir".to_string(),
            description: "List directory entries with metadata (name, type, size, modified time, \
                          extension)."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list (default: working directory)"
                    },
                    "maxEntries": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum entries to return (default 200)"
                    },
                    "sortBy": {
                        "type": "string",
                        "enum": ["name", "modified"],
                        "description": "Sort order: 'name' (default) or 'modified' (newest first)"
                    }
                },
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "read"])
            .permission("filesystem.read")
            .side_effect(SideEffectLevel::None)
            .concurrency_safe(true)
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "List the immediate contents of a directory before drilling into specific \
                     files.",
                    "Use `listDir` to understand repository structure, confirm filenames, and \
                     narrow the search space before calling read or edit tools. Prefer it over \
                     shell directory listings because it returns structured metadata.",
                )
                .caveat(
                    "It only returns a shallow directory listing; use `findFiles` for recursive \
                     discovery.",
                )
                .example(
                    "Check which packages or source folders exist under the current workspace.",
                )
                .prompt_tag("filesystem"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(ctx.cancel())?;

        let args: ListDirArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for listDir", e))?;
        let started_at = Instant::now();
        let path = match args.path {
            Some(path) => resolve_path(ctx, &path)?,
            None => ctx.working_dir().to_path_buf(),
        };
        let max_entries = args.max_entries.unwrap_or(200);
        let sort_by = args.sort_by.unwrap_or_default();

        let mut entries: Vec<DirEntry> = Vec::new();
        let mut truncated = false;
        let read_dir = fs::read_dir(&path).map_err(|e| {
            AstrError::io(format!("failed reading directory '{}'", path.display()), e)
        })?;

        for entry in read_dir {
            check_cancel(ctx.cancel())?;
            if entries.len() >= max_entries {
                truncated = true;
                break;
            }
            let entry = entry?;
            let file_type = entry.file_type()?;
            let metadata = fs::metadata(entry.path()).ok();

            let extension = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string());

            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: file_type.is_dir(),
                is_file: file_type.is_file(),
                size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: metadata.and_then(|m| m.modified().ok()),
                extension,
            });
        }

        // 排序
        match sort_by {
            SortBy::Name => {
                // 目录优先，然后按名称排序
                entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                });
            },
            SortBy::Modified => {
                // 按修改时间降序（最新优先），目录优先
                entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => {
                        let a_time = a.modified.unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        let b_time = b.modified.unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        b_time.cmp(&a_time)
                    },
                });
            },
        }

        // 转换为 JSON
        let json_entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "isDir": e.is_dir,
                    "isFile": e.is_file,
                    "size": e.size,
                    "modified": e.modified.map(|t| {
                        // 简单的 ISO 8601 格式，不依赖 chrono
                        let duration = t.duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap_or_default();
                        let secs = duration.as_secs();
                        let datetime = time_at(secs);
                        datetime
                    }),
                    "extension": e.extension,
                })
            })
            .collect();

        // 空目录返回友好提示，避免空 JSON 触发 stop sequence
        let output = if json_entries.is_empty() {
            "Directory is empty.".to_string()
        } else {
            serde_json::to_string(&json_entries)
                .map_err(|e| AstrError::parse("failed to serialize listDir output", e))?
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "listDir".to_string(),
            ok: true,
            output,
            error: None,
            metadata: Some(json!({
                "path": path.to_string_lossy(),
                "count": json_entries.len(),
                "truncated": truncated,
                "sortBy": match sort_by {
                    SortBy::Name => "name",
                    SortBy::Modified => "modified",
                },
            })),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

    #[tokio::test]
    async fn list_dir_tool_lists_entries_with_metadata() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("a.txt"), "hello world")
            .await
            .expect("write should work");

        let tool = ListDirTool;
        let result = tool
            .execute(
                "tc-list-meta".to_string(),
                json!({"path": temp.path().to_string_lossy()}),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("listDir should succeed");

        assert!(result.ok);
        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["name"], "a.txt");
        assert_eq!(entries[0]["isFile"], true);
        assert_eq!(entries[0]["size"], 11); // "hello world" 的字节数
        assert_eq!(entries[0]["extension"], "txt");
        assert!(entries[0]["modified"].is_string());
    }

    #[tokio::test]
    async fn list_dir_tool_honors_max_entries() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("a.txt"), "x")
            .await
            .expect("write should work");
        tokio::fs::write(temp.path().join("b.txt"), "x")
            .await
            .expect("write should work");

        let tool = ListDirTool;
        let result = tool
            .execute(
                "tc-list-max".to_string(),
                json!({"path": temp.path().to_string_lossy(), "maxEntries": 1}),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("listDir should succeed");

        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(entries.len(), 1);
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn list_dir_tool_sorts_by_modified_time() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file1 = temp.path().join("old.txt");
        let file2 = temp.path().join("new.txt");

        tokio::fs::write(&file1, "old")
            .await
            .expect("write should work");
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        tokio::fs::write(&file2, "new")
            .await
            .expect("write should work");

        let tool = ListDirTool;
        let result = tool
            .execute(
                "tc-list-sort".to_string(),
                json!({
                    "path": temp.path().to_string_lossy(),
                    "sortBy": "modified"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("listDir should succeed");

        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        // 新文件应排在前面
        assert_eq!(entries[0]["name"], "new.txt");
        assert_eq!(entries[1]["name"], "old.txt");
    }

    #[tokio::test]
    async fn list_dir_tool_directories_first() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::create_dir(temp.path().join("zdir"))
            .await
            .expect("mkdir should work");
        tokio::fs::write(temp.path().join("afile.txt"), "x")
            .await
            .expect("write should work");

        let tool = ListDirTool;
        let result = tool
            .execute(
                "tc-list-dirs-first".to_string(),
                json!({"path": temp.path().to_string_lossy()}),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("listDir should succeed");

        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        // 目录应排在前面，即使名称字母顺序更靠后
        assert_eq!(entries[0]["name"], "zdir");
        assert_eq!(entries[0]["isDir"], true);
        assert_eq!(entries[1]["name"], "afile.txt");
    }
}
