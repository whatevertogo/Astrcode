//! # Grep 工具
//!
//! 实现 `grep` 工具，用于在文件或目录中搜索正则表达式匹配行。
//!
//! ## 设计要点
//!
//! - 支持递归搜索（`recursive: true`）和单层搜索
//! - 可配置大小写敏感和最大匹配数
//! - 支持 `offset` 分页，LLM 可迭代获取超出 `maxMatches` 的后续结果
//! - `GrepMatch` 增加 `match_text` 字段，精确提取匹配到的子串
//! - 空结果返回友好提示文本，避免空输出触发 stop sequence

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

use crate::tools::fs_common::{
    check_cancel, maybe_persist_large_tool_result, read_utf8_file, resolve_path,
};

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
    /// 跳过的匹配数量，用于分页获取后续结果。
    #[serde(default)]
    offset: Option<usize>,
}

/// 单次正则匹配的结果。
///
/// 包含文件路径、行号、完整行内容和精确匹配子串。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct GrepMatch {
    /// 文件路径（绝对路径字符串）。
    file: String,
    /// 匹配行号（1-based）。
    line_no: usize,
    /// 完整行内容。
    line: String,
    /// 精确匹配到的子串，帮助 LLM 快速定位关键片段。
    #[serde(skip_serializing_if = "Option::is_none")]
    match_text: Option<String>,
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
                    "pattern": {
                        "type": "string",
                        "description": "Rust regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in"
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Search subdirectories recursively"
                    },
                    "caseInsensitive": { "type": "boolean" },
                    "maxMatches": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of matches to return (default 100)"
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Number of matches to skip for pagination"
                    }
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
                     drawing conclusions from broad searches. When `truncated` is true and \
                     `has_more` is true, use `offset` to fetch the next page.",
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
        let offset = args.offset.unwrap_or(0);
        let mut matches = Vec::new();
        let mut total_in_page = 0usize;
        let mut skipped_files = 0usize;
        let mut hit_limit = false;

        let files = collect_candidate_files(&path, args.recursive, ctx.cancel())?;
        for file in files {
            check_cancel(ctx.cancel())?;

            let content = match read_utf8_file(&file).await {
                Ok(content) => content,
                Err(error) => {
                    warn!("grep: skipping '{}': {error}", file.display());
                    skipped_files += 1;
                    continue;
                },
            };

            for (index, line) in content.lines().enumerate() {
                check_cancel(ctx.cancel())?;
                if regex.is_match(line) {
                    total_in_page += 1;
                    // 跳过 offset 之前的匹配
                    if total_in_page <= offset {
                        continue;
                    }
                    let match_text = extract_match_text(&regex, line);
                    matches.push(GrepMatch {
                        file: file.to_string_lossy().to_string(),
                        line_no: index + 1,
                        line: line.to_string(),
                        match_text,
                    });
                    if matches.len() >= max_matches {
                        hit_limit = true;
                        break;
                    }
                }
            }

            if hit_limit {
                break;
            }
        }

        // `hit_limit` 表示达到 maxMatches 上限，可能有更多结果未扫描。
        // 不继续扫描剩余文件以获取精确 total_found，因为遍历整个仓库
        // 可能很慢（尤其是递归搜索）。
        let has_more = hit_limit;

        // 空结果返回友好提示文本，避免空输出触发 stop sequence
        let output = if matches.is_empty() {
            if offset > 0 {
                "No more matches found (all remaining results after offset have been exhausted)."
                    .to_string()
            } else {
                "No matches found for the given pattern.".to_string()
            }
        } else {
            serde_json::to_string(&matches)
                .map_err(|e| AstrError::parse("failed to serialize grep matches", e))?
        };

        // 溢出存盘检查
        // TODO: session_dir() 尚未注入 ToolContext，暂时使用 force_inline 跳过存盘。
        // 需要在 runtime 层将 ~/<project>/sessions/<id>/ 路径注入 ToolContext。
        let final_output =
            maybe_persist_large_tool_result(ctx.working_dir(), &tool_call_id, &output, true);
        let is_persisted = final_output.starts_with("<persisted-output>");

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "grep".to_string(),
            ok: true,
            output: final_output,
            error: None,
            metadata: Some(json!({
                "pattern": args.pattern,
                "returned": matches.len(),
                "has_more": has_more,
                "truncated": has_more || is_persisted,
                "skipped_files": skipped_files,
                "offset_applied": offset,
            })),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: has_more,
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
        if entry.file_type()?.is_file() {
            files.push(entry.path());
        }
    }
    Ok(files)
}

/// 从匹配行中提取精确匹配到的子串。
///
/// 当正则包含捕获组时返回第一个捕获组的内容，
/// 否则返回整个匹配到的子串。
/// 这帮助 LLM 快速定位长行中的关键片段。
fn extract_match_text(re: &regex::Regex, line: &str) -> Option<String> {
    re.captures(line).and_then(|caps| {
        if caps.len() > 1 {
            caps.get(1).map(|m| m.as_str().to_string())
        } else {
            caps.get(0).map(|m| m.as_str().to_string())
        }
    })
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
        assert!(!result.output.starts_with("No matches found"));
        let matches: Vec<GrepMatch> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_no, 1);
        assert_eq!(matches[1].line_no, 3);
        assert_eq!(
            matches[0].file,
            canonical_tool_path(&file).to_string_lossy().to_string()
        );
        assert_eq!(matches[0].match_text, Some("pub fn".to_string()));
    }

    #[tokio::test]
    async fn grep_returns_friendly_text_when_no_matches() {
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
        assert_eq!(result.output, "No matches found for the given pattern.");
    }

    #[tokio::test]
    async fn grep_supports_offset_pagination() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("lib.rs");
        tokio::fs::write(
            &file,
            "pub fn a() {}\npub fn b() {}\npub fn c() {}\npub fn d() {}\n",
        )
        .await
        .expect("seed write should work");
        let tool = GrepTool;

        // 第一页：maxMatches=2
        let result = tool
            .execute(
                "tc-grep-p1".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": file.to_string_lossy(),
                    "maxMatches": 2
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should succeed");

        let matches: Vec<GrepMatch> =
            serde_json::from_str(&result.output).expect("output should be valid json");
        assert_eq!(matches.len(), 2);
        assert!(result.truncated);
        let meta = result.metadata.as_ref().expect("metadata should exist");
        assert_eq!(meta["has_more"], json!(true));

        // 第二页：offset=2, 使用更大的 maxMatches 以确保不触发 hit_limit
        let result2 = tool
            .execute(
                "tc-grep-p2".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": file.to_string_lossy(),
                    "maxMatches": 10,
                    "offset": 2
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should succeed");

        let matches2: Vec<GrepMatch> =
            serde_json::from_str(&result2.output).expect("output should be valid json");
        assert_eq!(matches2.len(), 2);
        assert_eq!(matches2[0].line_no, 3);
        assert_eq!(matches2[1].line_no, 4);
        assert!(!result2.truncated);
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
            .expect_err("grep should fail for invalid regex");

        let msg = format!("{err}");
        assert!(msg.contains("invalid regex"));
    }

    #[tokio::test]
    async fn grep_offset_exhausted_returns_friendly_text() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("lib.rs");
        tokio::fs::write(&file, "pub fn a() {}\npub fn b() {}\n")
            .await
            .expect("seed write should work");
        let tool = GrepTool;

        let result = tool
            .execute(
                "tc-grep-exhausted".to_string(),
                json!({
                    "pattern": "pub fn",
                    "path": file.to_string_lossy(),
                    "offset": 5
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("grep should succeed");

        assert_eq!(
            result.output,
            "No more matches found (all remaining results after offset have been exhausted)."
        );
    }

    #[test]
    fn extract_match_text_returns_first_capture_group() {
        let re = regex::Regex::new(r"fn\s+(\w+)").unwrap();
        let text = extract_match_text(&re, "pub fn greet(name: &str)");
        assert_eq!(text, Some("greet".to_string()));
    }

    #[test]
    fn extract_match_text_returns_full_match_when_no_groups() {
        let re = regex::Regex::new(r"pub fn").unwrap();
        let text = extract_match_text(&re, "pub fn main()");
        assert_eq!(text, Some("pub fn".to_string()));
    }
}
