//! # ReadFile 工具
//!
//! 实现 `readFile` 工具，用于读取 UTF-8 文本文件内容。
//!
//! ## 设计要点
//!
//! - 默认最大返回 20,000 字符（context window 友好值）
//! - 截断点位于 UTF-8 字符边界
//! - 支持 `offset`（行偏移）和 `limit`（行数限制）参数
//! - 返回 metadata 包含原始大小和截断标记

use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    time::Instant,
};

use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::tools::fs_common::{check_cancel, resolve_path};

/// ReadFile 工具实现。
///
/// 读取 UTF-8 文本文件，支持按行偏移和字符预算。
#[derive(Default)]
pub struct ReadFileTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileArgs {
    path: PathBuf,
    /// 最大返回字符数，默认 20,000。
    #[serde(default)]
    max_chars: Option<usize>,
    /// 起始行号（0-based），用于跳过文件头部。
    #[serde(default)]
    offset: Option<usize>,
    /// 最多返回的行数，与 offset 配合使用。
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "readFile".to_string(),
            description: "Read a UTF-8 text file. Supports line-based offset/limit for targeted \
                          reads."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
                    },
                    "maxChars": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum characters to return (default 20000)"
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Starting line number (0-based). Skips lines before this offset."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of lines to read from the offset."
                    }
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
                    "Read the exact contents of a text file when you need authoritative code or \
                     config context.",
                    "Use `readFile` after you have identified the right path with `findFiles` or \
                     `grep`. Use `offset` and `limit` to read specific sections of large files.",
                )
                .caveat(
                    "Large files are truncated by `maxChars`. Use `offset` to read further into a \
                     file.",
                )
                .example(
                    "After grep shows a symbol at line 420, use offset=400 and limit=50 to read \
                     that section.",
                )
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
        let max_chars = args.max_chars.unwrap_or(20_000);
        let path = resolve_path(ctx, &args.path)?;

        let file = fs::File::open(&path)
            .map_err(|e| AstrError::io(format!("failed opening file '{}'", path.display()), e))?;
        let total_bytes = file
            .metadata()
            .map_err(|e| {
                AstrError::io(
                    format!("failed reading metadata for '{}'", path.display()),
                    e,
                )
            })?
            .len() as usize;
        let reader = BufReader::new(file);

        let (text, total_lines, truncated) = if args.offset.is_some() || args.limit.is_some() {
            read_lines_range(reader, args.offset.unwrap_or(0), args.limit, max_chars)
        } else {
            read_file_full(reader, max_chars)
        }?;

        let meta = if args.offset.is_some() || args.limit.is_some() {
            json!({
                "path": path.to_string_lossy(),
                "bytes": total_bytes,
                "total_lines": total_lines,
                "offset": args.offset.unwrap_or(0),
                "limit": args.limit,
                "truncated": truncated,
            })
        } else {
            json!({
                "path": path.to_string_lossy(),
                "bytes": total_bytes,
                "truncated": truncated,
            })
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "readFile".to_string(),
            ok: true,
            output: text,
            error: None,
            metadata: Some(meta),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

/// 读取文件的前 max_chars 个字符。
fn read_file_full(reader: BufReader<fs::File>, max_chars: usize) -> Result<(String, usize, bool)> {
    let mut output = String::new();
    for line_result in reader.lines() {
        let line = line_result.map_err(|e| AstrError::io("failed reading file line", e))?;
        let remaining = max_chars.saturating_sub(output.chars().count());
        if remaining == 0 {
            return Ok((output, 0, true));
        }
        if !output.is_empty() {
            output.push('\n');
        }
        let take = remaining.min(line.chars().count());
        let boundary = char_count_to_byte_offset(&line, take);
        output.push_str(&line[..boundary]);
        if boundary < line.len() {
            return Ok((output, 0, true));
        }
    }
    Ok((output, 0, false))
}

/// 将字符数量转换为字节偏移量。
///
/// `floor_char_boundary(n)` 的参数是字节位置而非字符数量，
/// 因此不能直接用于"取前 N 个字符"的场景。
fn char_count_to_byte_offset(s: &str, char_count: usize) -> usize {
    s.char_indices()
        .nth(char_count)
        .map_or(s.len(), |(idx, _)| idx)
}

/// 按行范围读取：跳过 offset 行，最多读取 limit 行。
///
/// 返回 `(output, total_line_count, truncated)`，其中 `total_line_count`
/// 是文件的实际总行数（即使超出 limit 也会继续计数）。
fn read_lines_range(
    reader: BufReader<fs::File>,
    offset: usize,
    limit: Option<usize>,
    max_chars: usize,
) -> Result<(String, usize, bool)> {
    let mut output = String::new();
    let mut line_count = 0usize;
    let mut lines_read = 0usize;
    let max_lines = limit.unwrap_or(usize::MAX);

    for line_result in reader.lines() {
        let line = line_result.map_err(|e| AstrError::io("failed reading file line", e))?;
        line_count += 1;

        if line_count <= offset {
            continue;
        }

        // 已读够 limit 行，跳过但继续计数以获取准确总行数
        if lines_read >= max_lines {
            continue;
        }

        let remaining = max_chars.saturating_sub(output.chars().count());
        if remaining == 0 {
            return Ok((output, line_count, true));
        }
        if !output.is_empty() {
            output.push('\n');
        }
        let take = remaining.min(line.chars().count());
        let boundary = char_count_to_byte_offset(&line, take);
        output.push_str(&line[..boundary]);
        lines_read += 1;
        // 单行超出字符预算
        if boundary < line.len() {
            return Ok((output, line_count, true));
        }
    }

    // 自然 EOF：只有字符预算耗尽才算截断
    let truncated = output.chars().count() >= max_chars && line_count > offset;
    Ok((output, line_count, truncated))
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
                json!({ "path": file.to_string_lossy(), "maxChars": 3 }),
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
                json!({ "path": file.to_string_lossy(), "maxChars": 1 }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert_eq!(result.output, "你");
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn read_file_supports_offset_and_limit() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "line0\nline1\nline2\nline3\nline4\n")
            .await
            .expect("write should work");
        let tool = ReadFileTool;

        let result = tool
            .execute(
                "tc-offset".to_string(),
                json!({ "path": file.to_string_lossy(), "offset": 2, "limit": 2 }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert_eq!(result.output, "line2\nline3");
        assert!(!result.truncated);
        let meta = result.metadata.expect("metadata should exist");
        assert_eq!(meta["total_lines"], json!(5));
        assert_eq!(meta["limit"], json!(2));
    }
}
