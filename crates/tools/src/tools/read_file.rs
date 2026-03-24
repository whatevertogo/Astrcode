use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::tools::fs_common::{check_cancel, resolve_path};
use astrcode_core::{AstrError, Result, Tool, ToolContext, ToolDefinition, ToolExecutionResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

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
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(&ctx.cancel, "readFile")?;

        let args: ReadFileArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for readFile", e))?;
        let started_at = Instant::now();
        let max_bytes = args.max_bytes.unwrap_or(64 * 1024);
        let path = resolve_path(ctx, &args.path)?;

        let bytes = fs::read(&path)
            .map_err(|e| AstrError::io(format!("failed reading file '{}'", path.display()), e))?;

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
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

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
                &test_tool_context_for(temp.path()),
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
}
