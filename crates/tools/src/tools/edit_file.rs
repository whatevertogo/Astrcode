use crate::tools::fs_common::{check_cancel, read_utf8_file, resolve_path, write_text_file};
use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Default)]
pub struct EditFileTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditFileArgs {
    path: PathBuf,
    old_str: String,
    new_str: String,
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }

    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        count += 1;
        start += pos + 1;
    }
    count
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
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(&ctx.cancel, "editFile")?;

        let args: EditFileArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for editFile", e))?;
        if args.old_str.is_empty() {
            return Err(AstrError::Validation("oldStr cannot be empty".to_string()));
        }

        let started_at = Instant::now();
        let path = resolve_path(ctx, &args.path)?;
        let content = read_utf8_file(&path).await?;
        let occurrences = count_occurrences(&content, &args.old_str);

        if occurrences == 0 {
            return Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "editFile".to_string(),
                ok: false,
                output: String::new(),
                error: Some("oldStr not found in file".to_string()),
                metadata: Some(json!({
                    "path": path.to_string_lossy(),
                })),
                duration_ms: started_at.elapsed().as_millis() as u64,
                truncated: false,
            });
        }

        if occurrences > 1 {
            return Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "editFile".to_string(),
                ok: false,
                output: String::new(),
                error: Some(format!(
                    "oldStr appears {occurrences} times, must be unique to edit safely"
                )),
                metadata: Some(json!({
                    "path": path.to_string_lossy(),
                })),
                duration_ms: started_at.elapsed().as_millis() as u64,
                truncated: false,
            });
        }

        let replaced = content.replacen(&args.old_str, &args.new_str, 1);
        write_text_file(&path, &replaced, false).await?;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "editFile".to_string(),
            ok: true,
            output: format!("replaced 1 occurrence in {}", path.to_string_lossy()),
            error: None,
            metadata: Some(json!({
                "path": path.to_string_lossy(),
            })),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

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
            json!(file.to_string_lossy().to_string())
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
        assert!(result.error.unwrap_or_default().contains("appears 2 times"));
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
        assert!(result.error.unwrap_or_default().contains("appears 2 times"));
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
            ctx.cancel.cancel();
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

    #[tokio::test]
    async fn edit_file_supports_relative_paths() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("hello.txt"), "hello world")
            .await
            .expect("seed write should work");

        let tool = EditFileTool;
        let result = tool
            .execute(
                "tc-edit-relative".to_string(),
                json!({
                    "path": "hello.txt",
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(temp.path().join("hello.txt"))
            .await
            .expect("file should be readable");
        assert_eq!(content, "world world");
    }
}
