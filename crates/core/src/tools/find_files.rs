use anyhow::{Context, Result};
use async_trait::async_trait;
use glob::glob;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::action::{ToolDefinition, ToolExecutionResult};
use crate::tools::fs_common::{check_cancel, json_output, resolve_path};
use crate::tools::Tool;

#[derive(Default)]
pub struct FindFilesTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindFilesArgs {
    pattern: String,
    #[serde(default)]
    root: Option<PathBuf>,
    #[serde(default)]
    max_results: Option<usize>,
}

#[async_trait]
impl Tool for FindFilesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "findFiles".to_string(),
            description: "Find files matching a glob pattern. Use ** for recursive search."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "root": { "type": "string" },
                    "maxResults": { "type": "integer", "minimum": 1 }
                },
                "required": ["pattern"],
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
        check_cancel(&cancel, "findFiles")?;

        let args: FindFilesArgs =
            serde_json::from_value(args).context("invalid args for findFiles")?;
        let started_at = Instant::now();
        let root = match args.root {
            Some(root) => resolve_path(&root)?,
            None => std::env::current_dir().context("failed to resolve current directory")?,
        };
        let max_results = args.max_results.unwrap_or(200);
        let full_pattern = root
            .join(&args.pattern)
            .to_string_lossy()
            .replace('\\', "/");
        let entries = glob(&full_pattern)
            .with_context(|| format!("failed to parse glob pattern '{}'", full_pattern))?;

        let mut paths = Vec::new();
        let mut truncated = false;
        for entry in entries {
            check_cancel(&cancel, "findFiles")?;
            let path = entry.with_context(|| format!("failed matching '{}'", full_pattern))?;
            if path.is_file() {
                let resolved = resolve_path(&path)?;
                paths.push(resolved.to_string_lossy().to_string());
                if paths.len() >= max_results {
                    truncated = true;
                    break;
                }
            }
        }

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "findFiles".to_string(),
            ok: true,
            output: json_output(&paths)?,
            error: None,
            metadata: Some(json!({
                "pattern": args.pattern,
                "root": root.to_string_lossy(),
                "count": paths.len(),
                "truncated": truncated,
            })),
            duration_ms: started_at.elapsed().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestEnvGuard;

    #[tokio::test]
    async fn find_files_matches_direct_glob() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("a.txt"), "a")
            .await
            .expect("seed write should work");
        let tool = FindFilesTool;

        let result = tool
            .execute(
                "tc-find-direct".to_string(),
                json!({
                    "pattern": "*.txt",
                    "root": temp.path().to_string_lossy()
                }),
                CancellationToken::new(),
            )
            .await
            .expect("findFiles should execute");

        assert!(result.ok);
        let paths: Vec<String> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0],
            temp.path().join("a.txt").to_string_lossy().to_string()
        );
    }

    #[tokio::test]
    async fn find_files_matches_recursive_glob() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let nested = temp.path().join("nested");
        tokio::fs::create_dir_all(&nested)
            .await
            .expect("mkdir should work");
        tokio::fs::write(nested.join("lib.rs"), "fn main() {}")
            .await
            .expect("seed write should work");
        let tool = FindFilesTool;

        let result = tool
            .execute(
                "tc-find-recursive".to_string(),
                json!({
                    "pattern": "**/*.rs",
                    "root": temp.path().to_string_lossy()
                }),
                CancellationToken::new(),
            )
            .await
            .expect("findFiles should execute");

        assert!(result.ok);
        let paths: Vec<String> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(paths.len(), 1);
    }

    #[tokio::test]
    async fn find_files_returns_empty_list_when_no_match_exists() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let tool = FindFilesTool;

        let result = tool
            .execute(
                "tc-find-empty".to_string(),
                json!({
                    "pattern": "*.txt",
                    "root": temp.path().to_string_lossy()
                }),
                CancellationToken::new(),
            )
            .await
            .expect("findFiles should execute");

        assert!(result.ok);
        let paths: Vec<String> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert!(paths.is_empty());
    }

    #[tokio::test]
    async fn find_files_truncates_at_max_results() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("a.txt"), "a")
            .await
            .expect("seed write should work");
        tokio::fs::write(temp.path().join("b.txt"), "b")
            .await
            .expect("seed write should work");
        let tool = FindFilesTool;

        let result = tool
            .execute(
                "tc-find-truncate".to_string(),
                json!({
                    "pattern": "*.txt",
                    "root": temp.path().to_string_lossy(),
                    "maxResults": 1
                }),
                CancellationToken::new(),
            )
            .await
            .expect("findFiles should execute");

        assert!(result.ok);
        let paths: Vec<String> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(paths.len(), 1);
    }

    #[tokio::test]
    async fn find_files_returns_interrupted_error_when_cancelled() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("a.txt"), "a")
            .await
            .expect("seed write should work");
        let tool = FindFilesTool;
        let cancel = CancellationToken::new();
        cancel.cancel();

        let err = tool
            .execute(
                "tc-find-cancel".to_string(),
                json!({
                    "pattern": "*.txt",
                    "root": temp.path().to_string_lossy()
                }),
                cancel,
            )
            .await
            .expect_err("findFiles should fail");

        assert!(err.to_string().contains("findFiles interrupted"));
    }

    #[tokio::test]
    async fn find_files_supports_relative_root_and_reports_absolute_metadata() {
        let guard = TestEnvGuard::new();
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("a.txt"), "a")
            .await
            .expect("seed write should work");
        guard.set_current_dir(temp.path());

        let tool = FindFilesTool;
        let result = tool
            .execute(
                "tc-find-relative".to_string(),
                json!({
                    "pattern": "*.txt",
                    "root": "."
                }),
                CancellationToken::new(),
            )
            .await
            .expect("findFiles should execute");

        assert!(result.ok);
        assert_eq!(
            result.metadata.expect("metadata should exist")["root"],
            json!(temp.path().to_string_lossy().to_string())
        );
    }
}
