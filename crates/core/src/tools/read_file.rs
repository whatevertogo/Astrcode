use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::action::{ToolDefinition, ToolExecutionResult};
use crate::tools::fs_common::{check_cancel, resolve_path};
use crate::tools::Tool;

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

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolExecutionResult> {
        check_cancel(&cancel, "readFile")?;

        let args: ReadFileArgs =
            serde_json::from_value(args).context("invalid args for readFile")?;
        let started_at = Instant::now();
        let max_bytes = args.max_bytes.unwrap_or(64 * 1024);
        let path = resolve_path(&args.path)?;

        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed reading file '{}'", path.display()))?;

        let truncated = bytes.len() > max_bytes;
        let content_bytes = if truncated {
            &bytes[..max_bytes]
        } else {
            &bytes[..]
        };

        let content = String::from_utf8_lossy(content_bytes).to_string();

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
            duration_ms: started_at.elapsed().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_file_tool_reads_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "hello from read_file")
            .await
            .expect("write should work");

        let tool = ReadFileTool;
        let result = tool
            .execute(
                "tc2".to_string(),
                json!({ "path": file.to_string_lossy() }),
                CancellationToken::new(),
            )
            .await
            .expect("readFile should succeed");

        assert!(result.ok);
        assert_eq!(result.output, "hello from read_file");
        assert_eq!(
            result.metadata.expect("metadata should exist")["path"],
            json!(file.to_string_lossy().to_string())
        );
    }
}
