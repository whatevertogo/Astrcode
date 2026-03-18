use astrcode_core::{
    AstrError, CancelToken, Result, Tool, ToolContext, ToolDefinition, ToolExecutionResult,
};
use async_trait::async_trait;
use log::warn;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;
use walkdir::WalkDir;

use crate::tools::fs_common::{check_cancel, json_output, read_utf8_file, resolve_path};

#[derive(Default)]
pub struct GrepTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrepArgs {
    pattern: String,
    path: PathBuf,
    #[serde(default)]
    recursive: bool,
    #[serde(default)]
    case_insensitive: bool,
    #[serde(default)]
    max_matches: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct GrepMatch {
    file: String,
    line_no: usize,
    line: String,
}

#[async_trait]
impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "Search for a regex pattern in a file or directory. Returns matching lines with file path and line number.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" },
                    "caseInsensitive": { "type": "boolean" },
                    "maxMatches": { "type": "integer", "minimum": 1 }
                },
                "required": ["pattern", "path"],
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
        check_cancel(&ctx.cancel, "grep")?;

        let args: GrepArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for grep", e))?;
        let path = resolve_path(ctx, &args.path)?;
        let started_at = Instant::now();
        let regex = RegexBuilder::new(&args.pattern)
            .case_insensitive(args.case_insensitive)
            .build()
            .map_err(|error| AstrError::ToolError {
                name: "grep".to_string(),
                reason: format!("invalid regex: {}", error),
            })?;
        let max_matches = args.max_matches.unwrap_or(100);
        let mut matches = Vec::new();
        let mut truncated = false;
        let mut skipped_files = 0usize;

        let files = collect_candidate_files(&path, args.recursive, &ctx.cancel)?;
        for file in files {
            check_cancel(&ctx.cancel, "grep")?;

            let content = match read_utf8_file(&file).await {
                Ok(content) => content,
                Err(error) => {
                    warn!("grep: skipping '{}': {}", file.display(), error);
                    skipped_files += 1;
                    continue;
                }
            };

            for (index, line) in content.lines().enumerate() {
                check_cancel(&ctx.cancel, "grep")?;
                if regex.is_match(line) {
                    matches.push(GrepMatch {
                        file: file.to_string_lossy().to_string(),
                        line_no: index + 1,
                        line: line.to_string(),
                    });
                    if matches.len() >= max_matches {
                        truncated = true;
                        break;
                    }
                }
            }

            if truncated {
                break;
            }
        }

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "grep".to_string(),
            ok: true,
            output: json_output(&matches)?,
            error: None,
            metadata: Some(json!({
                "pattern": args.pattern,
                "total_matches": matches.len(),
                "truncated": truncated,
                "skipped_files": skipped_files,
            })),
            duration_ms: started_at.elapsed().as_millis(),
        })
    }
}

fn collect_candidate_files(
    path: &Path,
    recursive: bool,
    cancel: &CancelToken,
) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    if !path.is_dir() {
        return Err(AstrError::Validation(format!(
            "path is neither a file nor directory: {}",
            path.display()
        )));
    }

    if recursive {
        let mut files = Vec::new();
        for entry in WalkDir::new(path) {
            check_cancel(cancel, "grep")?;
            let entry = entry.map_err(|e| {
                AstrError::io(
                    format!("failed walking '{}'", path.display()),
                    std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
                )
            })?;
            if entry.file_type().is_file() {
                files.push(entry.path().to_path_buf());
            }
        }
        return Ok(files);
    }

    let mut files = Vec::new();
    let read_dir = std::fs::read_dir(path)
        .map_err(|e| AstrError::io(format!("failed reading directory '{}'", path.display()), e))?;
    for entry in read_dir {
        check_cancel(cancel, "grep")?;
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file() {
            files.push(entry.path());
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

    #[tokio::test]
    async fn grep_finds_matches_with_line_numbers() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("lib.rs");
        tokio::fs::write(&file, "pub fn a() {}\nlet x = 1;\npub fn b() {}\n")
            .await
            .expect("seed write should work");
        let tool = GrepTool;

        let result = tool
            .execute(
                "tc-grep-found".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": file.to_string_lossy()
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should execute");

        assert!(result.ok);
        let matches: Vec<GrepMatch> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_no, 1);
        assert_eq!(matches[1].line_no, 3);
        assert_eq!(matches[0].file, file.to_string_lossy().to_string());
    }

    #[tokio::test]
    async fn grep_returns_empty_list_when_no_matches_exist() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("lib.rs");
        tokio::fs::write(&file, "let x = 1;\n")
            .await
            .expect("seed write should work");
        let tool = GrepTool;

        let result = tool
            .execute(
                "tc-grep-empty".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": file.to_string_lossy()
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should execute");

        assert!(result.ok);
        let matches: Vec<GrepMatch> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn grep_errors_for_invalid_regex() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("lib.rs");
        tokio::fs::write(&file, "let x = 1;\n")
            .await
            .expect("seed write should work");
        let tool = GrepTool;

        let err = tool
            .execute(
                "tc-grep-invalid".to_string(),
                json!({
                    "pattern": "(",
                    "path": file.to_string_lossy()
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect_err("grep should fail");

        assert!(err.to_string().contains("invalid regex"));
    }

    #[tokio::test]
    async fn grep_matches_case_insensitively_when_requested() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("lib.rs");
        tokio::fs::write(&file, "Pub Fn a() {}\n")
            .await
            .expect("seed write should work");
        let tool = GrepTool;

        let result = tool
            .execute(
                "tc-grep-ci".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": file.to_string_lossy(),
                    "caseInsensitive": true
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should execute");

        assert!(result.ok);
        let matches: Vec<GrepMatch> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(matches.len(), 1);
    }

    #[tokio::test]
    async fn grep_searches_recursively() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let nested = temp.path().join("nested");
        tokio::fs::create_dir_all(&nested)
            .await
            .expect("mkdir should work");
        let file = nested.join("lib.rs");
        tokio::fs::write(&file, "pub fn a() {}\n")
            .await
            .expect("seed write should work");
        let tool = GrepTool;

        let result = tool
            .execute(
                "tc-grep-recursive".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": temp.path().to_string_lossy(),
                    "recursive": true
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should execute");

        assert!(result.ok);
        let matches: Vec<GrepMatch> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(matches.len(), 1);
    }

    #[tokio::test]
    async fn grep_returns_interrupted_error_when_cancelled() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("lib.rs");
        tokio::fs::write(&file, "pub fn a() {}\n")
            .await
            .expect("seed write should work");
        let tool = GrepTool;
        let cancel = {
            let ctx = test_tool_context_for(temp.path());
            ctx.cancel.cancel();
            ctx
        };

        let err = tool
            .execute(
                "tc-grep-cancel".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": file.to_string_lossy()
                }),
                &cancel,
            )
            .await
            .expect_err("grep should fail");

        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn collect_candidate_files_honors_cancellation_during_recursive_walk() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let nested = temp.path().join("nested");
        std::fs::create_dir_all(&nested).expect("mkdir should work");
        std::fs::write(nested.join("lib.rs"), "pub fn a() {}\n").expect("seed write should work");

        let cancel = {
            let ctx = test_tool_context_for(temp.path());
            ctx.cancel.cancel();
            ctx
        };

        let err = collect_candidate_files(temp.path(), true, &cancel.cancel)
            .expect_err("recursive walk should fail when cancelled");

        assert!(err.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn grep_supports_relative_paths_and_reports_skipped_files() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        tokio::fs::write(temp.path().join("good.rs"), "pub fn a() {}\n")
            .await
            .expect("seed write should work");
        tokio::fs::write(temp.path().join("bad.bin"), [0xff, 0xfe, 0xfd])
            .await
            .expect("seed write should work");

        let tool = GrepTool;
        let result = tool
            .execute(
                "tc-grep-relative".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": "."
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should execute");

        assert!(result.ok);
        let matches: Vec<GrepMatch> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(matches.len(), 1);
        assert_eq!(
            result.metadata.expect("metadata should exist")["skipped_files"],
            json!(1)
        );
    }

    #[tokio::test]
    async fn grep_errors_for_missing_paths() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let tool = GrepTool;

        let err = tool
            .execute(
                "tc-grep-missing".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": "missing"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect_err("missing paths should fail");

        assert!(err
            .to_string()
            .contains("path is neither a file nor directory"));
    }
}
