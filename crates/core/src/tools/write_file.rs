use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::action::{ToolDefinition, ToolExecutionResult};
use crate::tools::fs_common::{check_cancel, resolve_path, write_text_file};
use crate::tools::Tool;

#[derive(Default)]
pub struct WriteFileTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteFileArgs {
    path: PathBuf,
    content: String,
    #[serde(default)]
    create_dirs: bool,
}

#[async_trait]
impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "writeFile".to_string(),
            description: "Write UTF-8 text to a file, creating or overwriting it.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "createDirs": { "type": "boolean" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolExecutionResult> {
        check_cancel(&cancel, "writeFile")?;

        let args: WriteFileArgs =
            serde_json::from_value(args).context("invalid args for writeFile")?;
        let started_at = Instant::now();
        let path = resolve_path(&args.path)?;
        let bytes = write_text_file(&path, &args.content, args.create_dirs).await?;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "writeFile".to_string(),
            ok: true,
            output: format!("wrote {bytes} bytes"),
            error: None,
            metadata: Some(json!({
                "path": path.to_string_lossy(),
                "bytes": bytes,
            })),
            duration_ms: started_at.elapsed().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::fs_common::env_lock_for_tests;

    #[tokio::test]
    async fn write_file_creates_new_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        let tool = WriteFileTool;

        let result = tool
            .execute(
                "tc-write-new".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "content": "hello"
                }),
                CancellationToken::new(),
            )
            .await
            .expect("writeFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "hello");
        assert_eq!(
            result.metadata.expect("metadata should exist")["path"],
            json!(file.to_string_lossy().to_string())
        );
    }

    #[tokio::test]
    async fn write_file_overwrites_existing_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "old")
            .await
            .expect("seed write should work");
        let tool = WriteFileTool;

        let result = tool
            .execute(
                "tc-write-overwrite".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "content": "new"
                }),
                CancellationToken::new(),
            )
            .await
            .expect("writeFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "new");
    }

    #[tokio::test]
    async fn write_file_creates_parent_directories_when_requested() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("nested").join("hello.txt");
        let tool = WriteFileTool;

        let result = tool
            .execute(
                "tc-write-create-dirs".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "content": "hello",
                    "createDirs": true
                }),
                CancellationToken::new(),
            )
            .await
            .expect("writeFile should execute");

        assert!(result.ok);
        assert!(file.exists());
    }

    #[tokio::test]
    async fn write_file_errors_when_parent_directory_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("nested").join("hello.txt");
        let tool = WriteFileTool;

        let err = tool
            .execute(
                "tc-write-missing-parent".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "content": "hello"
                }),
                CancellationToken::new(),
            )
            .await
            .expect_err("writeFile should fail");

        assert!(err.to_string().contains("failed writing file"));
    }

    #[tokio::test]
    async fn write_file_returns_interrupted_error_when_cancelled() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        let tool = WriteFileTool;
        let cancel = CancellationToken::new();
        cancel.cancel();

        let err = tool
            .execute(
                "tc-write-cancel".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "content": "hello"
                }),
                cancel,
            )
            .await
            .expect_err("writeFile should fail");

        assert!(err.to_string().contains("writeFile interrupted"));
    }

    #[tokio::test]
    async fn write_file_supports_relative_paths() {
        let _guard = env_lock_for_tests().lock().expect("env lock should work");
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let previous = std::env::current_dir().expect("cwd should resolve");
        std::env::set_current_dir(temp.path()).expect("set cwd should work");

        let tool = WriteFileTool;
        let result = tool
            .execute(
                "tc-write-relative".to_string(),
                json!({
                    "path": "hello.txt",
                    "content": "hello"
                }),
                CancellationToken::new(),
            )
            .await
            .expect("writeFile should execute");

        std::env::set_current_dir(previous).expect("restore cwd should work");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(temp.path().join("hello.txt"))
            .await
            .expect("file should be readable");
        assert_eq!(content, "hello");
    }
}
