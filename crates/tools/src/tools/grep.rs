//! # Grep 工具
//!
//! 实现 `grep` 工具，用于在文件或目录中搜索正则表达式匹配行。
//!
//! ## 设计要点
//!
//! - 支持递归搜索（`recursive: true`）和单层搜索
//! - 可配置大小写敏感和最大匹配数
//! - 默认最多返回 100 条匹配，避免超大输出
//! - 无法读取的文件（如二进制文件）会被跳过并记录警告

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use astrcode_core::{
    AstrError, CancelToken, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext,
    ToolDefinition, ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use log::warn;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::json;
use walkdir::WalkDir;

use crate::tools::fs_common::{check_cancel, json_output, read_utf8_file, resolve_path};

/// Grep 工具实现。
///
/// 在指定路径下搜索包含正则表达式匹配的文件内容。
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

/// 单次正则匹配的结果。
///
/// 包含文件名、行号（1-based）和完整行内容。
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
            description: "Search for a regex pattern in a file or directory. Returns matching \
                          lines with file path and line number."
                .to_string(),
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

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "read", "search"])
            .permission("filesystem.read")
            .side_effect(SideEffectLevel::None)
            .concurrency_safe(true)
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "Search file contents by regex when you need to locate code, config keys, or \
                     repeated text patterns.",
                    "Use `grep` after scoping the search path. It is the fastest way to answer \
                     where something is defined or referenced before opening specific files.",
                )
                .caveat(
                    "Regex patterns can over-match; narrow the path or cap `maxMatches` before \
                     drawing conclusions from broad searches.",
                )
                .example(
                    "Find all references to a symbol, config key, or error string inside a module \
                     or repository subtree.",
                )
                .prompt_tag("search")
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

        let files = collect_candidate_files(&path, args.recursive, ctx.cancel())?;
        for file in files {
            check_cancel(ctx.cancel())?;

            let content = match read_utf8_file(&file).await {
                Ok(content) => content,
                Err(error) => {
                    warn!("grep: skipping '{}': {}", file.display(), error);
                    skipped_files += 1;
                    continue;
                },
            };

            for (index, line) in content.lines().enumerate() {
                check_cancel(ctx.cancel())?;
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
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
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
            check_cancel(cancel)?;
            let entry = entry.map_err(|e| {
                AstrError::io(
                    format!("failed walking '{}'", path.display()),
                    std::io::Error::other(e.to_string()),
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
        check_cancel(cancel)?;
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
    use crate::test_support::{canonical_tool_path, test_tool_context_for};

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
        assert_eq!(
            matches[0].file,
            canonical_tool_path(&file).to_string_lossy().to_string()
        );
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
}
