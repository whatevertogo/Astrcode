//! # ApplyPatch 工具
//!
//! 实现 `apply_patch` 工具，用于以 unified diff 格式批量修改多个文件。
//!
//! ## 设计要点
//!
//! - 支持标准 unified diff 格式（类似 `git diff` 输出）
//! - 一次调用可对多个文件执行 add/update/delete 操作
//! - 严格上下文匹配：hunk 中的上下文行必须与文件内容完全匹配
//! - 自动生成变更报告，包含每个文件的 diff 统计
//!
//! ## 与 editFile 的区别
//!
//! `editFile` 基于字符串替换（要求 oldStr 唯一匹配），`apply_patch` 基于
//! unified diff 行级补丁。当需要对多个文件做小改动时，`apply_patch` 的
//! 输入格式对 LLM 更自然、更紧凑。

use std::time::Instant;

use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::builtin_tools::fs_common::{
    build_text_change_report, check_cancel, read_utf8_file, resolve_path, write_text_file,
};

/// ApplyPatch 工具实现。
#[derive(Default)]
pub struct ApplyPatchTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplyPatchArgs {
    patch: String,
}

// ── 内部数据结构 ──

#[derive(Debug)]
struct FilePatch {
    old_path: Option<String>,
    new_path: Option<String>,
    hunks: Vec<Hunk>,
}

#[derive(Debug)]
struct Hunk {
    old_start: usize,
    _old_count: usize,
    _new_start: usize,
    _new_count: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Add(String),
    Delete(String),
}

#[derive(Debug)]
struct FileChange {
    change_type: String,
    path: String,
    applied: bool,
    summary: String,
    error: Option<String>,
}

struct HunkResolution {
    content_start: usize,
    line_is_removal: Vec<bool>,
    add_lines: Vec<String>,
}

// ── Patch 解析 ──

fn parse_patch(patch: &str) -> Result<Vec<FilePatch>> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut file_patches: Vec<FilePatch> = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];

        // 跳过空行、注释、diff --git 和 git 元数据行
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("new file mode ")
            || line.starts_with("deleted file mode ")
            || line.starts_with("rename ")
            || line.starts_with("similarity ")
            || line.starts_with("copy ")
        {
            i += 1;
            continue;
        }

        if line.starts_with("--- ") {
            let old_path = strip_diff_prefix(line.strip_prefix("--- ").unwrap());
            i += 1;

            if i < lines.len() && lines[i].starts_with("+++ ") {
                let new_path = strip_diff_prefix(lines[i].strip_prefix("+++ ").unwrap());
                i += 1;
                let hunks = parse_hunks(&lines, &mut i)?;

                file_patches.push(FilePatch {
                    old_path: if old_path == "/dev/null" {
                        None
                    } else {
                        Some(old_path.to_string())
                    },
                    new_path: if new_path == "/dev/null" {
                        None
                    } else {
                        Some(new_path.to_string())
                    },
                    hunks,
                });
            } else {
                return Err(AstrError::Validation(
                    "patch format error: expected '+++ new_path' after '--- old_path'".into(),
                ));
            }
        } else {
            return Err(AstrError::Validation(format!(
                "patch format error: unexpected line '{}'",
                line.chars().take(30).collect::<String>()
            )));
        }
    }

    if file_patches.is_empty() {
        return Err(AstrError::Validation(
            "patch does not contain any file changes (no '---' lines found)".into(),
        ));
    }

    Ok(file_patches)
}

fn strip_diff_prefix(s: &str) -> &str {
    if s.starts_with("/dev/null") {
        return "/dev/null";
    }
    let trimmed = s.split('\t').next().unwrap_or(s);
    trimmed
        .strip_prefix("a/")
        .or_else(|| trimmed.strip_prefix("b/"))
        .unwrap_or(trimmed)
}

fn parse_hunks(lines: &[&str], i: &mut usize) -> Result<Vec<Hunk>> {
    let mut hunks = Vec::new();

    while *i < lines.len() {
        let line = lines[*i];

        if line.starts_with("--- ") || line.starts_with("diff ") {
            break;
        }

        if line.starts_with("@@") {
            let (old_start, old_count, new_start, new_count) = parse_hunk_header(line)?;
            *i += 1;

            let mut hunk_lines: Vec<HunkLine> = Vec::new();
            while *i < lines.len()
                && !lines[*i].starts_with("@@")
                && !lines[*i].starts_with("--- ")
                && !lines[*i].starts_with("diff ")
            {
                let l = lines[*i];
                if l.is_empty() {
                    hunk_lines.push(HunkLine::Context(String::new()));
                    *i += 1;
                } else {
                    match l.chars().next().unwrap() {
                        ' ' => {
                            hunk_lines.push(HunkLine::Context(l.chars().skip(1).collect()));
                            *i += 1;
                        },
                        '+' => {
                            hunk_lines.push(HunkLine::Add(l.chars().skip(1).collect()));
                            *i += 1;
                        },
                        '-' => {
                            hunk_lines.push(HunkLine::Delete(l.chars().skip(1).collect()));
                            *i += 1;
                        },
                        '\\' => {
                            *i += 1; // "\ No newline at end of file"
                        },
                        _ => {
                            hunk_lines.push(HunkLine::Context(l.to_string()));
                            *i += 1;
                        },
                    }
                }
            }

            hunks.push(Hunk {
                old_start,
                _old_count: old_count,
                _new_start: new_start,
                _new_count: new_count,
                lines: hunk_lines,
            });
        } else {
            *i += 1;
        }
    }

    Ok(hunks)
}

fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize, usize)> {
    let content = header
        .strip_prefix("@@")
        .and_then(|s| s.rsplit_once("@@"))
        .map(|(inner, _)| inner.trim())
        .ok_or_else(|| AstrError::Validation(format!("invalid hunk header: {header}")))?;

    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(AstrError::Validation(format!(
            "invalid hunk header (too few parts): {header}"
        )));
    }

    let (old_start, old_count) = parse_range(parts[0], "old")?;
    let (new_start, new_count) = parse_range(parts[1], "new")?;

    Ok((old_start, old_count, new_start, new_count))
}

fn parse_range(s: &str, kind: &str) -> Result<(usize, usize)> {
    let inner = s
        .strip_prefix('-')
        .or_else(|| s.strip_prefix('+'))
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "invalid {kind} range in hunk header: expected -/+ prefix, got '{s}'"
            ))
        })?;

    if let Some((start, count)) = inner.split_once(',') {
        let start: usize = start
            .parse()
            .map_err(|_| AstrError::Validation(format!("invalid {kind} range start: {start}")))?;
        let count: usize = count
            .parse()
            .map_err(|_| AstrError::Validation(format!("invalid {kind} range count: {count}")))?;
        Ok((start, count))
    } else {
        let start: usize = inner
            .parse()
            .map_err(|_| AstrError::Validation(format!("invalid {kind} range: {inner}")))?;
        Ok((start, if start == 0 { 0 } else { 1 }))
    }
}

// ── Hunk 应用 ──

fn text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.lines().map(String::from).collect()
}

fn apply_hunks(
    content_lines: &[String],
    hunks: &[Hunk],
) -> std::result::Result<Vec<String>, String> {
    let mut result = content_lines.to_vec();

    let mut resolved_hunks: Vec<HunkResolution> = Vec::new();
    for (idx, hunk) in hunks.iter().enumerate() {
        let pos = find_context_match(&result, hunk).ok_or_else(|| {
            format!(
                "hunk #{} (line ~{}) failed to apply: context mismatch",
                idx + 1,
                hunk.old_start
            )
        })?;
        resolved_hunks.push(resolve_hunk(hunk, pos));
    }

    // 从后向前应用，避免行偏移影响后续 hunk
    resolved_hunks.sort_by_key(|r| r.content_start);
    resolved_hunks.reverse();

    for resolution in &resolved_hunks {
        let mut to_remove: Vec<usize> = resolution
            .line_is_removal
            .iter()
            .enumerate()
            .filter(|&(_, &is_del)| is_del)
            .map(|(i, _)| resolution.content_start + i)
            .collect();
        to_remove.reverse();

        for idx in &to_remove {
            if *idx < result.len() {
                result.remove(*idx);
            }
        }

        let insert_pos = {
            let first_del_offset = resolution
                .line_is_removal
                .iter()
                .position(|&b| b)
                .unwrap_or(resolution.line_is_removal.len());
            resolution.content_start + first_del_offset
        };

        for (add_idx, line) in resolution.add_lines.iter().enumerate() {
            let pos = insert_pos.min(result.len());
            result.insert(pos + add_idx, line.clone());
        }
    }

    Ok(result)
}

fn resolve_hunk(hunk: &Hunk, content_start: usize) -> HunkResolution {
    let mut line_is_removal: Vec<bool> = Vec::new();
    let mut add_lines: Vec<String> = Vec::new();

    for hunk_line in &hunk.lines {
        match hunk_line {
            HunkLine::Context(_) => line_is_removal.push(false),
            HunkLine::Delete(_) => line_is_removal.push(true),
            HunkLine::Add(line) => add_lines.push(line.clone()),
        }
    }

    HunkResolution {
        content_start,
        line_is_removal,
        add_lines,
    }
}

fn find_context_match(content_lines: &[String], hunk: &Hunk) -> Option<usize> {
    let pattern: Vec<&str> = hunk
        .lines
        .iter()
        .filter_map(|l| match l {
            HunkLine::Context(s) | HunkLine::Delete(s) => Some(s.as_str()),
            HunkLine::Add(_) => None,
        })
        .collect();

    if pattern.is_empty() {
        return Some(content_lines.len());
    }

    let anchor = if hunk.old_start == 0 {
        0
    } else {
        hunk.old_start.saturating_sub(1)
    };

    if try_match_at(content_lines, &pattern, anchor) {
        return Some(anchor);
    }

    let search_range = pattern.len().max(10);
    let lower = anchor.saturating_sub(search_range);
    for offset in (lower..anchor).rev() {
        if try_match_at(content_lines, &pattern, offset) {
            return Some(offset);
        }
    }

    let upper_limit = content_lines.len().saturating_sub(pattern.len());
    ((anchor + 1)..=upper_limit).find(|&offset| try_match_at(content_lines, &pattern, offset))
}

fn try_match_at(content_lines: &[String], pattern: &[&str], start: usize) -> bool {
    if start + pattern.len() > content_lines.len() {
        return false;
    }
    pattern
        .iter()
        .enumerate()
        .all(|(i, &p)| content_lines.get(start + i).is_some_and(|line| line == p))
}

// ── 文件级操作 ──

async fn apply_file_patch(file_patch: &FilePatch, ctx: &ToolContext) -> FileChange {
    // 确定目标路径
    let target_path_str = file_patch
        .new_path
        .clone()
        .or_else(|| file_patch.old_path.clone());

    let target_path_str = match target_path_str {
        Some(p) => p,
        None => {
            return FileChange {
                change_type: "error".into(),
                path: "unknown".into(),
                applied: false,
                summary: "patch specifies neither old nor new path".into(),
                error: Some("patch specifies neither old nor new path".into()),
            };
        },
    };

    let is_new_file = file_patch.old_path.is_none();
    let is_delete = file_patch.new_path.is_none();
    let change_type = if is_new_file {
        "created"
    } else if is_delete {
        "deleted"
    } else {
        "updated"
    };

    if let Err(e) = check_cancel(ctx.cancel()) {
        return FileChange {
            change_type: change_type.into(),
            path: target_path_str.clone(),
            applied: false,
            summary: e.to_string(),
            error: Some(e.to_string()),
        };
    }

    let target_path = match resolve_path(ctx, std::path::Path::new(&target_path_str)) {
        Ok(p) => p,
        Err(e) => {
            return FileChange {
                change_type: change_type.into(),
                path: target_path_str.clone(),
                applied: false,
                summary: format!("failed to resolve path: {e}"),
                error: Some(e.to_string()),
            };
        },
    };

    let original_content = if target_path.exists() {
        match read_utf8_file(&target_path).await {
            Ok(content) => Some(content),
            Err(e) => {
                return FileChange {
                    change_type: change_type.into(),
                    path: target_path_str.clone(),
                    applied: false,
                    summary: format!("failed to read file: {e}"),
                    error: Some(format!("failed to read existing file: {e}")),
                };
            },
        }
    } else if is_new_file {
        None
    } else {
        return FileChange {
            change_type: change_type.into(),
            path: target_path_str.clone(),
            applied: false,
            summary: format!("file does not exist: {}", target_path_str),
            error: Some(format!(
                "file does not exist: {} - use '--- /dev/null' or writeFile instead",
                target_path_str
            )),
        };
    };

    if is_delete {
        if let Err(e) = std::fs::remove_file(&target_path) {
            return FileChange {
                change_type: "deleted".into(),
                path: target_path_str.clone(),
                applied: false,
                summary: format!("failed to delete {}: {e}", target_path.display()),
                error: Some(format!("failed to delete file: {e}")),
            };
        }
        return FileChange {
            change_type: "deleted".into(),
            path: target_path_str.clone(),
            applied: true,
            summary: format!("deleted {}", target_path.display()),
            error: None,
        };
    }

    let content_lines = original_content
        .as_deref()
        .map(text_lines)
        .unwrap_or_default();

    let result_lines = match apply_hunks(&content_lines, &file_patch.hunks) {
        Ok(lines) => lines,
        Err(e) => {
            return FileChange {
                change_type: change_type.into(),
                path: target_path_str.clone(),
                applied: false,
                summary: format!("failed to apply patch to {}: {e}", target_path.display()),
                error: Some(format!("failed to apply hunk: {e}")),
            };
        },
    };

    if let Err(e) = check_cancel(ctx.cancel()) {
        return FileChange {
            change_type: change_type.into(),
            path: target_path_str.clone(),
            applied: false,
            summary: e.to_string(),
            error: Some(e.to_string()),
        };
    }

    // 保留原始文件的尾部换行符（如果有且结果非空）
    let trailing_newline = original_content
        .as_deref()
        .is_some_and(|s| s.ends_with('\n'));
    let new_content = if trailing_newline && !result_lines.is_empty() {
        format!("{}\n", result_lines.join("\n"))
    } else {
        result_lines.join("\n")
    };
    let report = build_text_change_report(
        &target_path,
        change_type,
        original_content.as_deref(),
        &new_content,
    );

    if let Err(e) = write_text_file(&target_path, &new_content, false).await {
        return FileChange {
            change_type: change_type.into(),
            path: target_path_str.clone(),
            applied: false,
            summary: format!("failed to write {}: {e}", target_path.display()),
            error: Some(format!("failed to write file: {e}")),
        };
    }

    FileChange {
        change_type: change_type.into(),
        path: target_path_str.clone(),
        applied: true,
        summary: report.summary,
        error: None,
    }
}

// ── Tool trait 实现 ──

#[async_trait]
impl Tool for ApplyPatchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "apply_patch".to_string(),
            description: "Apply a unified diff patch to one or more files. Supports creating (--- \
                          /dev/null), updating, and deleting (+++ /dev/null) files."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "Unified diff patch text containing one or more file changes. \
                                        Use '--- /dev/null' to create, '+++ /dev/null' to delete."
                    }
                },
                "required": ["patch"],
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "write", "patch", "diff"])
            .permission("filesystem.write")
            .side_effect(SideEffectLevel::Workspace)
            .prompt(
                ToolPromptMetadata::new(
                    "Apply a unified diff patch across one or more files.",
                    "Use `apply_patch` for coordinated multi-file changes using standard unified \
                     diff format (like `git diff` output).",
                )
                .caveat(
                    "Hunks must apply cleanly. If the patch doesn't apply, fall back to \
                     `editFile` with explicit oldStr/newStr.",
                )
                .caveat("Use '--- /dev/null' to create new files, '+++ /dev/null' to delete.")
                .example(
                    "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n fn hello() {\n-    \
                     println!(\"old\");\n+    println!(\"new\");\n }",
                )
                .prompt_tag("filesystem")
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

        let args: ApplyPatchArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for apply_patch", e))?;

        let started_at = Instant::now();

        if args.patch.is_empty() {
            return make_error_result(&tool_call_id, "patch cannot be empty", started_at);
        }

        let file_patches = match parse_patch(&args.patch) {
            Ok(fp) => fp,
            Err(e) => {
                return make_error_result(&tool_call_id, &e.to_string(), started_at);
            },
        };

        let total_files = file_patches.len();

        let mut results: Vec<FileChange> = Vec::new();
        for file_patch in &file_patches {
            check_cancel(ctx.cancel())?;
            let result = apply_file_patch(file_patch, ctx).await;
            results.push(result);
        }

        let applied = results.iter().filter(|r| r.applied).count();
        let failed = total_files - applied;

        let (ok, output, error) = if failed == 0 {
            (
                true,
                format!("apply_patch: {applied}/{total_files} files changed successfully"),
                None,
            )
        } else if applied == 0 {
            (
                false,
                format!("apply_patch: all {total_files} file(s) failed to apply"),
                Some(format!("{failed} file(s) failed to apply")),
            )
        } else {
            (
                false,
                format!(
                    "apply_patch: {applied}/{total_files} files changed, {failed} failed (with \
                     partial changes committed)"
                ),
                Some(format!("{failed} file(s) failed to apply")),
            )
        };

        let metadata = build_apply_patch_metadata(&results, applied, failed);

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "apply_patch".to_string(),
            ok,
            output,
            error,
            metadata: Some(metadata),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

fn make_error_result(
    tool_call_id: &str,
    error_msg: &str,
    started_at: Instant,
) -> Result<ToolExecutionResult> {
    Ok(ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: "apply_patch".to_string(),
        ok: false,
        output: String::new(),
        error: Some(error_msg.to_string()),
        metadata: Some(json!({
            "filesChanged": 0,
            "filesApplied": 0,
            "filesFailed": 0,
        })),
        duration_ms: started_at.elapsed().as_millis() as u64,
        truncated: false,
    })
}

fn build_apply_patch_metadata(
    results: &[FileChange],
    applied: usize,
    failed: usize,
) -> serde_json::Value {
    let file_results: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let mut obj = json!({
                "path": r.path,
                "changeType": r.change_type,
                "applied": r.applied,
                "summary": r.summary,
            });
            if let Some(err) = &r.error {
                obj.as_object_mut()
                    .unwrap()
                    .insert("error".to_string(), json!(err));
            }
            obj
        })
        .collect();

    json!({
        "filesChanged": applied,
        "filesApplied": applied,
        "filesFailed": failed,
        "files": file_results,
    })
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

    #[test]
    fn parse_single_file_single_hunk() {
        let patch = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\nfn foo() {\n-    \
                     old_code();\n+    new_code();\n}\n";

        let patches = parse_patch(patch).expect("should parse");
        assert_eq!(patches.len(), 1);
        let fp = &patches[0];
        assert_eq!(fp.old_path.as_deref(), Some("src/main.rs"));
        assert_eq!(fp.new_path.as_deref(), Some("src/main.rs"));
        assert_eq!(fp.hunks.len(), 1);

        let hunk = &fp.hunks[0];
        assert_eq!(hunk.old_start, 1);
    }

    #[test]
    fn parse_new_file() {
        let patch = "--- /dev/null\n+++ b/new_file.txt\n@@ -0,0 +1,2 @@\n+line one\n+line two\n";

        let patches = parse_patch(patch).expect("should parse");
        assert!(patches[0].old_path.is_none());
        assert_eq!(patches[0].new_path.as_deref(), Some("new_file.txt"));
    }

    #[test]
    fn parse_delete_file() {
        let patch = "--- a/old_file.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-line one\n-line two\n";

        let patches = parse_patch(patch).expect("should parse");
        assert!(patches[0].new_path.is_none());
        assert_eq!(patches[0].old_path.as_deref(), Some("old_file.txt"));
    }

    #[test]
    fn parse_git_diff_prefix() {
        let patch = "diff --git a/src/foo.rs b/src/foo.rs\n--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ \
                     -1 +1,2 @@\nexisting()\n+new_line()\n";

        let patches = parse_patch(patch).expect("should parse");
        assert_eq!(patches[0].old_path.as_deref(), Some("src/foo.rs"));
    }

    #[test]
    fn parse_empty_error() {
        let err = parse_patch("").unwrap_err();
        assert!(err.to_string().contains("not contain any file changes"));
    }

    #[test]
    fn apply_hunk_replace_line() {
        let content = vec![
            "fn foo() {".to_string(),
            "    old();".to_string(),
            "}".to_string(),
        ];

        let hunk = Hunk {
            old_start: 1,
            _old_count: 3,
            _new_start: 1,
            _new_count: 3,
            lines: vec![
                HunkLine::Context("fn foo() {".into()),
                HunkLine::Delete("    old();".into()),
                HunkLine::Add("    new();".into()),
                HunkLine::Context("}".into()),
            ],
        };

        let result = apply_hunks(&content, &[hunk]).expect("should apply");
        assert_eq!(result[1], "    new();");
    }

    #[tokio::test]
    async fn apply_patch_creates_new_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tool = ApplyPatchTool;

        let patch = "--- /dev/null\n+++ b/hello.rs\n@@ -0,0 +1,3 @@\n+fn main() {\n+    \
                     println!(\"hello\");\n+}\n";

        let result = tool
            .execute(
                "tc-patch-create".into(),
                json!({ "patch": patch }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("should execute");

        assert!(result.ok, "should succeed: {}", result.output);
        assert!(temp.path().join("hello.rs").exists());
    }

    #[tokio::test]
    async fn apply_patch_updates_existing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file = temp.path().join("test.rs");
        tokio::fs::write(&file, "fn foo() {\n    old();\n}\n")
            .await
            .expect("seed write");

        let tool = ApplyPatchTool;
        let patch = "--- a/test.rs\n+++ b/test.rs\n@@ -1,3 +1,3 @@\nfn foo() {\n-    old();\n+    \
                     new();\n}\n";

        let result = tool
            .execute(
                "tc-patch-update".into(),
                json!({ "patch": patch }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("should execute");

        assert!(result.ok, "should succeed: {}", result.output);
        let content = tokio::fs::read_to_string(&file).await.expect("readable");
        assert!(content.contains("new()"));
        assert!(!content.contains("old()"), "old should be removed");
    }

    #[tokio::test]
    async fn apply_patch_empty_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tool = ApplyPatchTool;

        let result = tool
            .execute(
                "tc-patch-empty".into(),
                json!({ "patch": "" }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("should return result");

        assert!(!result.ok);
    }

    #[tokio::test]
    async fn apply_patch_invalid_format() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tool = ApplyPatchTool;

        let result = tool
            .execute(
                "tc-patch-invalid".into(),
                json!({ "patch": "not a valid patch" }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("should return result");

        assert!(!result.ok);
    }

    #[tokio::test]
    async fn apply_patch_preserves_trailing_newline() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file = temp.path().join("test.rs");
        // 文件以换行符结尾
        tokio::fs::write(&file, "fn foo() {\n    old();\n}\n")
            .await
            .expect("seed write");

        let tool = ApplyPatchTool;
        let patch = "--- a/test.rs\n+++ b/test.rs\n@@ -1,3 +1,3 @@\nfn foo() {\n-    old();\n+    \
                     new();\n}\n";

        let result = tool
            .execute(
                "tc-patch-trailing".into(),
                json!({ "patch": patch }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("should execute");

        assert!(result.ok, "should succeed: {}", result.output);
        let content = tokio::fs::read_to_string(&file).await.expect("readable");
        // 验证尾部换行符被保留
        assert!(
            content.ends_with('\n'),
            "trailing newline should be preserved"
        );
        assert!(content.contains("new()"));
    }
}
