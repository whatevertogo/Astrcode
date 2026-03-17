use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::tools::fs_common::{check_cancel, resolve_path};
use astrcode_core::{AstrError, Result, Tool, ToolContext, ToolDefinition, ToolExecutionResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

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

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(&ctx.cancel, "listDir")?;

        let args: ListDirArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for listDir", e))?;
        let started_at = Instant::now();
        let path = match args.path {
            Some(path) => resolve_path(ctx, &path)?,
            None => ctx.working_dir.clone(),
        };
        let max_entries = args.max_entries.unwrap_or(200);

        let mut entries = Vec::new();
        let read_dir = fs::read_dir(&path).map_err(|e| {
            AstrError::io(format!("failed reading directory '{}'", path.display()), e)
        })?;
        for entry in read_dir {
            if entries.len() >= max_entries {
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
                "count": entries.len()
            })),
            duration_ms: started_at.elapsed().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

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
            json!(temp.path().to_string_lossy().to_string())
        );
    }
}
