//! # EditFile 工具
//!
//! 实现 `editFile` 工具，用于在文件中进行精确的字符串替换。
//!
//! ## 安全机制
//!
//! - `oldStr` 必须在文件中**恰好出现一次**，否则拒绝编辑
//! - 支持重叠匹配检测（如在 `"ababa"` 中搜索 `"aba"` 会找到两个位置）
//! - 编辑前/后均检查取消标记，避免长文件操作无法中断
//!
//! ## 与 writeFile 的区别
//!
//! `writeFile` 用于完全覆盖，`editFile` 用于窄范围修改。
//! 优先使用 `editFile` 可以保持变更更小、更容易验证。

use crate::tools::fs_common::{
    build_text_change_report, check_cancel, read_utf8_file, resolve_path, write_text_file,
};
use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// EditFile 工具实现。
///
/// 在文件中查找唯一出现的 `oldStr` 并替换为 `newStr`。
/// 如果 `oldStr` 出现 0 次或多次，编辑被拒绝以防止意外修改。
#[derive(Default)]
pub struct EditFileTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditFileArgs {
    path: PathBuf,
    old_str: String,
    new_str: String,
}

/// 在 haystack 中查找 needle 的唯一出现位置。
///
/// **为什么需要重叠检测**: 如果只按 `needle.len()` 步进，对于 `"ababa"` 中搜索 `"aba"`
/// 会漏掉位置 2 的重叠匹配。edit_file 要求 oldStr 在文件中必须唯一，
/// 因此需要逐 UTF-8 标量步进来捕获所有可能的匹配位置。
/// 找到多个匹配时返回错误，拒绝编辑以防止意外修改错误的位置。
fn find_unique_occurrence(haystack: &str, needle: &str) -> Option<Result<usize>> {
    if needle.is_empty() {
        return None;
    }

    let mut first_match = None;
    let mut offset = 0usize;
    while let Some(relative_pos) = haystack[offset..].find(needle) {
        let absolute_pos = offset + relative_pos;
        if first_match.replace(absolute_pos).is_some() {
            return Some(Err(AstrError::Validation(
                "oldStr appears multiple times, must be unique to edit safely".to_string(),
            )));
        }

        // 步进一个 UTF-8 标量以检测重叠匹配
        let step = haystack[absolute_pos..]
            .chars()
            .next()
            .map_or(1, |c| c.len_utf8());
        offset = absolute_pos + step;
    }

    first_match.map(Ok)
}

#[async_trait]
impl Tool for EditFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "editFile".to_string(),
            description: "Replace a unique string in a file with new content. old_str must appear exactly once - if it appears zero or multiple times the edit is rejected to prevent unintended changes.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "oldStr": { "type": "string" },
                    "newStr": { "type": "string" }
                },
                "required": ["path", "oldStr", "newStr"],
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "write", "edit"])
            .permission("filesystem.write")
            .side_effect(SideEffectLevel::Workspace)
            .prompt(
                ToolPromptMetadata::new(
                    "Apply a narrow, safety-checked string replacement inside an existing file.",
                    "Use `editFile` when you know the exact old text and want a minimal change. It rejects ambiguous replacements, which makes it safer than rewriting a whole file for small edits.",
                )
                .caveat("`oldStr` must match exactly once; if the text is missing or duplicated, the edit is rejected.")
                .example("Rename a flag or replace one function body fragment without regenerating the whole file.")
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

        let args: EditFileArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for editFile", e))?;
        if args.old_str.is_empty() {
            return Err(AstrError::Validation("oldStr cannot be empty".to_string()));
        }

        let started_at = Instant::now();
        let path = resolve_path(ctx, &args.path)?;
        let content = read_utf8_file(&path).await?;
        check_cancel(ctx.cancel())?;
        let match_start = match find_unique_occurrence(&content, &args.old_str) {
            Some(Ok(pos)) => pos,
            Some(Err(_)) => {
                return make_edit_error_result(
                    &tool_call_id,
                    "oldStr appears multiple times, must be unique to edit safely",
                    &path,
                    started_at,
                );
            }
            None => {
                return make_edit_error_result(
                    &tool_call_id,
                    "oldStr not found in file",
                    &path,
                    started_at,
                );
            }
        };

        let match_end = match_start + args.old_str.len();
        let mut replaced =
            String::with_capacity(content.len() - args.old_str.len() + args.new_str.len());
        replaced.push_str(&content[..match_start]);
        replaced.push_str(&args.new_str);
        replaced.push_str(&content[match_end..]);
        let report = build_text_change_report(&path, "updated", Some(&content), &replaced);
        check_cancel(ctx.cancel())?;
        write_text_file(&path, &replaced, false).await?;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "editFile".to_string(),
            ok: true,
            output: report.summary,
            error: None,
            metadata: Some(report.metadata),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

/// 构建 editFile 失败时的统一响应。
fn make_edit_error_result(
    tool_call_id: &str,
    error: &str,
    path: &Path,
    started_at: Instant,
) -> std::result::Result<ToolExecutionResult, AstrError> {
    Ok(ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: "editFile".to_string(),
        ok: false,
        output: String::new(),
        error: Some(error.to_string()),
        metadata: Some(json!({
            "path": path.to_string_lossy(),
        })),
        duration_ms: started_at.elapsed().as_millis() as u64,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{canonical_tool_path, test_tool_context_for};

    #[tokio::test]
    async fn edit_file_replaces_unique_occurrence() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-ok".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "world world");
        assert_eq!(
            result.metadata.expect("metadata should exist")["path"],
            json!(canonical_tool_path(&file).to_string_lossy().to_string())
        );
    }

    #[tokio::test]
    async fn edit_file_refuses_when_old_str_missing() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-missing".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "missing",
                    "newStr": "world"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("not found"));
    }

    #[tokio::test]
    async fn edit_file_refuses_when_old_str_appears_multiple_times() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello hello")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-multi".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("multiple times"));
    }

    #[tokio::test]
    async fn edit_file_refuses_when_old_str_overlaps_itself() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "ababa")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-overlap".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "aba",
                    "newStr": "x"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("multiple times"));
    }

    #[tokio::test]
    async fn edit_file_returns_interrupted_error_when_cancelled() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;
        let cancel = {
            let ctx = test_tool_context_for(temp.path());
            ctx.cancel().cancel();
            ctx
        };

        let err = tool
            .execute(
                "tc-edit-cancel".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &cancel,
            )
            .await
            .expect_err("editFile should fail");

        assert!(err.to_string().contains("cancelled"));
    }
}
