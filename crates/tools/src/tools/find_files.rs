//! # FindFiles 工具
//!
//! 实现 `findFiles` 工具，用于基于 glob 模式查找文件。
//!
//! ## 设计要点
//!
//! - 使用 `glob` crate 进行模式匹配，支持 `**` 递归
//! - 路径沙箱检查：glob 模式不能逃逸工作目录
//! - 默认最多返回 200 条结果
//! - 返回结构化 JSON 数组，便于前端渲染

use crate::tools::fs_common::{check_cancel, json_output, resolve_path};
use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use glob::glob;
use serde::Deserialize;
use serde_json::json;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

/// FindFiles 工具实现。
///
/// 基于 glob 模式在工作目录内查找匹配的文件路径。
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

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "read", "search"])
            .permission("filesystem.read")
            .side_effect(SideEffectLevel::None)
            .concurrency_safe(true)
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "Find candidate files by glob when you know the filename pattern but not the exact path.",
                    "Use `findFiles` to narrow a workspace to a set of matching paths before calling `readFile` or `editFile`. It is better than shell globbing because it stays within the working directory and returns structured results.",
                )
                .caveat("Patterns must stay inside the workspace; use `**` explicitly when you need recursive matching.")
                .example("Locate all `Cargo.toml`, `package.json`, or `*.tsx` files under the repo.")
                .prompt_tag("search"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(ctx.cancel())?;

        let args: FindFilesArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for findFiles", e))?;
        let started_at = Instant::now();
        validate_glob_pattern(&args.pattern)?;
        let root = match args.root {
            Some(root) => resolve_path(ctx, &root)?,
            None => resolve_path(ctx, Path::new("."))?,
        };
        let max_results = args.max_results.unwrap_or(200);
        let full_pattern = root
            .join(&args.pattern)
            .to_string_lossy()
            .replace('\\', "/");
        let entries = glob(&full_pattern).map_err(|e| AstrError::ToolError {
            name: "findFiles".to_string(),
            reason: format!("failed to parse glob pattern '{}': {}", full_pattern, e),
        })?;

        let mut paths = Vec::new();
        let mut truncated = false;
        for entry in entries {
            check_cancel(ctx.cancel())?;
            let path = entry.map_err(|e| AstrError::ToolError {
                name: "findFiles".to_string(),
                reason: format!("failed matching '{}': {}", full_pattern, e),
            })?;
            if path.is_file() {
                let resolved = resolve_path(ctx, &path)?;
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
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

fn validate_glob_pattern(pattern: &str) -> Result<()> {
    if looks_like_windows_drive_relative_path(pattern) {
        return Err(AstrError::Validation(format!(
            "glob pattern '{}' must stay within the working directory",
            pattern
        )));
    }

    let path = Path::new(pattern);
    if path.is_absolute() {
        return Err(AstrError::Validation(format!(
            "glob pattern '{}' must stay within the working directory",
            pattern
        )));
    }

    for component in path.components() {
        match component {
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(AstrError::Validation(format!(
                    "glob pattern '{}' must stay within the working directory",
                    pattern
                )));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    Ok(())
}

fn looks_like_windows_drive_relative_path(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{canonical_tool_path, test_tool_context_for};

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
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("findFiles should execute");

        assert!(result.ok);
        let paths: Vec<String> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0],
            canonical_tool_path(temp.path().join("a.txt"))
                .to_string_lossy()
                .to_string()
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
                &test_tool_context_for(temp.path()),
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
                &test_tool_context_for(temp.path()),
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
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("findFiles should execute");

        assert!(result.ok);
        let paths: Vec<String> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(paths.len(), 1);
    }
}
