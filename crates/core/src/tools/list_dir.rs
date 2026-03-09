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
        cancel: CancellationToken,
    ) -> Result<ToolExecutionResult> {
        check_cancel(&cancel, "listDir")?;

        let args: ListDirArgs = serde_json::from_value(args).context("invalid args for listDir")?;
        let started_at = Instant::now();
        let path = match args.path {
            Some(path) => resolve_path(&path)?,
            None => std::env::current_dir().context("failed to resolve current directory")?,
        };
        let max_entries = args.max_entries.unwrap_or(200);

        let mut dir = tokio::fs::read_dir(&path)
            .await
            .with_context(|| format!("failed reading directory '{}'", path.display()))?;

        let mut entries = Vec::new();
        while let Some(entry) = dir.next_entry().await? {
            if entries.len() >= max_entries {
                break;
            }
            let file_type = entry.file_type().await?;
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
            output: serde_json::to_string(&entries)?,
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
                CancellationToken::new(),
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
