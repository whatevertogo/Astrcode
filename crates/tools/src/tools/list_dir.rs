//! # ListDir 工具
//!
//! 实现 `listDir` 工具，用于浅层列出目录内容。
//!
//! ## 设计要点
//!
//! - 仅返回一层目录/文件条目，不递归
//! - 每个条目返回 `name`、`isDir`、`isFile`
//! - 默认最多 200 条，超出标记 `truncated`
//! - 未指定路径时使用上下文工作目录

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::tools::fs_common::{check_cancel, resolve_path};
use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

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
}

#[async_trait]
impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "listDir".to_string(),
            description: "List directory entries with basic metadata.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "maxEntries": { "type": "integer", "minimum": 1 }
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
                    "List the immediate contents of a directory before drilling into specific files.",
                    "Use `listDir` to understand repository structure, confirm filenames, and narrow the search space before calling read or edit tools. Prefer it over shell directory listings because it returns structured metadata.",
                )
                .caveat("It only returns a shallow directory listing; use `findFiles` for recursive discovery.")
                .example("Check which packages or source folders exist under the current workspace.")
                .prompt_tag("filesystem"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(ctx.cancel(), "listDir")?;

        let args: ListDirArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for listDir", e))?;
        let started_at = Instant::now();
        let path = match args.path {
            Some(path) => resolve_path(ctx, &path)?,
            None => ctx.working_dir().clone(),
        };
        let max_entries = args.max_entries.unwrap_or(200);

        let mut entries = Vec::new();
        let mut truncated = false;
        let read_dir = fs::read_dir(&path).map_err(|e| {
            AstrError::io(format!("failed reading directory '{}'", path.display()), e)
        })?;
        for entry in read_dir {
            check_cancel(ctx.cancel(), "listDir")?;
            if entries.len() >= max_entries {
                truncated = true;
                break;
            }
            let entry = entry?;
            let file_type = entry.file_type()?;
            entries.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "isDir": file_type.is_dir(),
                "isFile": file_type.is_file(),
            }));
        }

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "listDir".to_string(),
            ok: true,
            output: serde_json::to_string(&entries)
                .map_err(|e| AstrError::parse("failed to serialize listDir output", e))?,
            error: None,
            metadata: Some(json!({
                "path": path,
                "count": entries.len(),
                "truncated": truncated,
            })),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{canonical_tool_path, test_tool_context_for};

    #[tokio::test]
    async fn list_dir_tool_lists_entries() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("a.txt"), "x")
            .await
            .expect("write should work");

        let tool = ListDirTool;
        let result = tool
            .execute(
                "tc3".to_string(),
                json!({"path": temp.path().to_string_lossy()}),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("listDir should succeed");

        assert!(result.ok);
        assert!(result.output.contains("a.txt"));
        assert_eq!(
            result.metadata.expect("metadata should exist")["path"],
            json!(canonical_tool_path(temp.path())
                .to_string_lossy()
                .to_string())
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
    }

    #[tokio::test]
    async fn list_dir_tool_errors_for_missing_path() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let tool = ListDirTool;

        let err = tool
            .execute(
                "tc-list-missing".to_string(),
                json!({"path": "missing"}),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect_err("missing paths should fail");

        assert!(err.to_string().contains("failed reading directory"));
    }

    #[tokio::test]
    async fn list_dir_tool_returns_cancelled_when_context_is_cancelled() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let ctx = {
            let ctx = test_tool_context_for(temp.path());
            ctx.cancel().cancel();
            ctx
        };
        let tool = ListDirTool;

        let err = tool
            .execute("tc-list-cancel".to_string(), json!({}), &ctx)
            .await
            .expect_err("cancelled listDir should fail");

        assert!(err.to_string().contains("cancelled"));
    }
}
