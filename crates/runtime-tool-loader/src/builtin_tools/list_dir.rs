//! # ListDir 工具
//!
//! 实现 `listDir` 工具，用于浅层列出目录内容。
//!
//! ## 设计要点
//!
//! - 仅返回一层目录/文件条目，不递归
//! - 每个条目返回 `name`、`type`（file/directory/symlink）、`size`、`modified`、
//!   `extension`（仅文件）
//! - 默认最多 200 条，超出标记 `truncated`
//! - 未指定路径时使用上下文工作目录
//! - 支持排序：按名称（默认）或按修改时间（最新优先）

use std::{fs, path::PathBuf, time::Instant};

use astrcode_core::{
    AstrError, Result, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use astrcode_protocol::capability::SideEffectLevel;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::builtin_tools::fs_common::{check_cancel, resolve_path};

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
    Size,
}

/// 目录条目信息。
#[derive(Debug, Clone)]
struct DirEntry {
    name: String,
    /// 条目类型：file / directory / symlink
    entry_type: String,
    size: u64,
    modified: Option<std::time::SystemTime>,
    /// 仅文件有扩展名，目录和符号链接不返回此字段
    extension: Option<String>,
}

#[async_trait]
impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "listDir".to_string(),
            description: concat!(
                "List immediate directory entries with metadata ",
                "(name, type, size, modified time, extension). ",
                "The `type` field is one of: file, directory, symlink. ",
                "The `extension` field is only present for files."
            )
            .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative directory path. Defaults to working directory if omitted."
                    },
                    "maxEntries": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum entries to return (default 200)."
                    },
                    "sortBy": {
                        "type": "string",
                        "enum": ["name", "modified", "size"],
                        "description": "Sort order: 'name' (alphabetical, default), 'modified' (newest first), or 'size' (largest first)."
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
                    "List directory entries as structured metadata (name/type/size/modified). The \
                     `type` field is \"file\", \"directory\", or \"symlink\". The `extension` \
                     field only appears for files. Returns one level only — use `path` to drill \
                     deeper. Directory `size` is always 0 on Windows; only file sizes are \
                     meaningful.",
                )
                .caveat(
                    "Truncated at maxEntries (default 200). When truncated, use a more specific \
                     path or `findFiles`.",
                )
                .example("List root: { }. List src/: { path: \"src\", sortBy: \"modified\" }")
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

            let entry_type = if file_type.is_dir() {
                "directory"
            } else if file_type.is_file() {
                "file"
            } else {
                "symlink"
            };

            let extension = if file_type.is_file() {
                entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            };

            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                entry_type: entry_type.to_string(),
                size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: metadata.and_then(|m| m.modified().ok()),
                extension,
            });
        }

        // 排序
        match sort_by {
            SortBy::Name => {
                // 目录优先，然后按名称排序
                entries.sort_by(
                    |a, b| match (a.entry_type.as_str(), b.entry_type.as_str()) {
                        ("directory", "file" | "symlink") => std::cmp::Ordering::Less,
                        ("file" | "symlink", "directory") => std::cmp::Ordering::Greater,
                        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    },
                );
            },
            SortBy::Modified => {
                // 按修改时间降序（最新优先），目录优先
                entries.sort_by(
                    |a, b| match (a.entry_type.as_str(), b.entry_type.as_str()) {
                        ("directory", "file" | "symlink") => std::cmp::Ordering::Less,
                        ("file" | "symlink", "directory") => std::cmp::Ordering::Greater,
                        _ => {
                            let a_time = a.modified.unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                            let b_time = b.modified.unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                            b_time.cmp(&a_time)
                        },
                    },
                );
            },
            SortBy::Size => {
                // 按文件大小降序（最大优先），目录优先
                entries.sort_by(
                    |a, b| match (a.entry_type.as_str(), b.entry_type.as_str()) {
                        ("directory", "file" | "symlink") => std::cmp::Ordering::Less,
                        ("file" | "symlink", "directory") => std::cmp::Ordering::Greater,
                        _ => b.size.cmp(&a.size),
                    },
                );
            },
        }

        // 转换为 JSON
        let json_entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                let mut obj = json!({
                    "name": e.name,
                    "type": e.entry_type,
                    "size": e.size,
                    "modified": e.modified.map(|t| {
                        // 这里返回真实 RFC3339 UTC 时间，便于排序和跨端展示保持一致。
                        DateTime::<Utc>::from(t).to_rfc3339()
                    }),
                });
                // 仅文件返回 extension，目录/符号链接不返回以避免 AI 误解 null 含义
                if let Some(ext) = &e.extension {
                    obj.as_object_mut()
                        .expect("obj is constructed above as object")
                        .insert("extension".to_string(), json!(ext));
                }
                obj
            })
            .collect();

        let output = serde_json::to_string(&json_entries)
            .map_err(|e| AstrError::parse("failed to serialize listDir output", e))?;
        let empty_message = json_entries
            .is_empty()
            .then_some("Directory is empty.".to_string());

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
                "message": empty_message,
                "sortBy": match sort_by {
                    SortBy::Name => "name",
                    SortBy::Modified => "modified",
                    SortBy::Size => "size",
                },
            })),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

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
        assert_eq!(entries[0]["type"], "file");
        assert_eq!(entries[0]["size"], 11); // "hello world" 的字节数
        assert_eq!(entries[0]["extension"], "txt");
        let modified = entries[0]["modified"]
            .as_str()
            .expect("modified timestamp should exist");
        assert!(
            DateTime::parse_from_rfc3339(modified).is_ok(),
            "modified should be RFC3339"
        );
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
        assert_eq!(entries[0]["type"], "directory");
        assert_eq!(entries[1]["name"], "afile.txt");
    }

    #[tokio::test]
    async fn list_dir_tool_returns_json_for_empty_directory() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let tool = ListDirTool;

        let result = tool
            .execute(
                "tc-list-empty".to_string(),
                json!({"path": temp.path().to_string_lossy()}),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("listDir should succeed");

        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&result.output).expect("output should remain valid json");
        assert!(entries.is_empty());
        let metadata = result.metadata.expect("metadata should exist");
        assert_eq!(metadata["message"], json!("Directory is empty."));
    }
}
