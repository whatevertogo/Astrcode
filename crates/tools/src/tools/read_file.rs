//! # ReadFile 工具
//!
//! 实现 `readFile` 工具，用于读取 UTF-8 文本文件内容。
//!
//! ## 设计要点
//!
//! - 默认最大读取 64KB，通过 `maxBytes` 参数可调整
//! - 截断点必须在 UTF-8 字符边界上，避免多字节字符被截断成无效字符串
//! - 返回 metadata 包含原始字节数和是否截断标记

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

/// ReadFile 工具实现。
///
/// 读取指定路径的 UTF-8 文本文件，支持按字节预算截断。
#[derive(Default)]
pub struct ReadFileTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileArgs {
    path: PathBuf,
    #[serde(default)]
    max_bytes: Option<usize>,
}

#[async_trait]
impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "readFile".to_string(),
            description: "Read a UTF-8 text file (truncated by maxBytes if provided).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "maxBytes": { "type": "integer", "minimum": 1 }
                },
                "required": ["path"],
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
                    "Read the exact contents of a text file when you need authoritative code or config context.",
                    "Use `readFile` after you have identified the right path with `findFiles` or `grep`. It is the primary source of truth for code analysis, debugging, and planning edits because it returns the file contents directly.",
                )
                .caveat("Large files can be truncated by `maxBytes`, so confirm whether truncation happened before making claims about the tail of a file.")
                .example("Open the implementation of a function or inspect a config file before editing it.")
                .prompt_tag("filesystem")
                .always_include(true),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(ctx.cancel())?;

        let args: ReadFileArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for readFile", e))?;
        let started_at = Instant::now();
        let max_bytes = args.max_bytes.unwrap_or(64 * 1024);
        let path = resolve_path(ctx, &args.path)?;

        let bytes = fs::read(&path)
            .map_err(|e| AstrError::io(format!("failed reading file '{}'", path.display()), e))?;
        let content = String::from_utf8_lossy(&bytes).to_string();
        let truncated = content.len() > max_bytes;
        let content = if truncated {
            // `maxBytes` 仍按字节预算工作，但截断点必须落在 UTF-8 字符边界上，
            // 否则中文/emoji 等多字节字符会在切片时被截断成无效字符串。
            let truncate_at = content.floor_char_boundary(max_bytes);
            content[..truncate_at].to_string()
        } else {
            content
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "readFile".to_string(),
            ok: true,
            output: content,
            error: None,
            metadata: Some(json!({
                "path": path,
                "bytes": bytes.len(),
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
    use crate::test_support::test_tool_context_for;

    #[tokio::test]
    async fn read_file_tool_marks_truncated_output() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "abcdef")
            .await
            .expect("write should work");

        let tool = ReadFileTool;
        let result = tool
            .execute(
                "tc3".to_string(),
                json!({ "path": file.to_string_lossy(), "maxBytes": 3 }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert_eq!(result.output, "abc");
        let metadata = result.metadata.expect("metadata should exist");
        assert_eq!(metadata["bytes"], json!(6));
        assert_eq!(metadata["truncated"], json!(true));
    }

    #[tokio::test]
    async fn read_file_tool_truncates_at_utf8_char_boundary() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "你好a")
            .await
            .expect("write should work");

        let tool = ReadFileTool;
        let result = tool
            .execute(
                "tc4".to_string(),
                json!({ "path": file.to_string_lossy(), "maxBytes": 4 }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert_eq!(result.output, "你");
        assert!(result.truncated);
    }
}
