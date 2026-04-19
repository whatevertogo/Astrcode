//! # EditFile 工具
//!
//! 实现 `editFile` 工具，用于在文件中进行精确的字符串替换。
//!
//! ## 安全机制
//!
//! - `oldStr` 必须在文件中**恰好出现一次**，否则拒绝编辑
//! - 支持重叠匹配检测（如在 `"ababa"` 中搜索 `"aba"` 会找到两个位置）
//! - 编辑前/后均检查取消标记，避免长文件操作无法中断
//!
//! ## 批量编辑
//!
//! 支持通过 `edits` 数组一次执行多个替换操作，按顺序应用。
//! 每个 edit 必须满足唯一匹配要求。
//!
//! ## 与 writeFile 的区别
//!
//! `writeFile` 用于完全覆盖，`editFile` 用于窄范围修改。
//! 优先使用 `editFile` 可以保持变更更小、更容易验证。

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use astrcode_core::{
    AstrError, Result, SideEffect, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::builtin_tools::fs_common::{
    build_text_change_report, capture_file_observation, check_cancel,
    ensure_not_canonical_session_plan_write_target, file_observation_matches, is_symlink,
    is_unc_path, load_file_observation, read_utf8_file, remember_file_observation, resolve_path,
    write_text_file,
};

/// 可编辑文件的最大大小（1 GiB）。
///
/// V8/Bun 字符串长度限制约为 2^30 字符（~10 亿）。对于典型的 ASCII/Latin-1 文件，
/// 1 字节 = 1 字符，因此 1 GiB 磁盘字节 ≈ 10 亿字符 ≈ 运行时字符串限制。
/// 多字节 UTF-8 文件每字符可能占用更多磁盘空间，但 1 GiB 是一个安全的字节级保护，
/// 可以防止 OOM 而不会过度限制。
const MAX_EDIT_FILE_SIZE: u64 = 1024 * 1024 * 1024; // 1 GiB

/// EditFile 工具实现。
///
/// 在文件中查找唯一出现的 `oldStr` 并替换为 `newStr`。
/// 如果 `oldStr` 出现 0 次或多次，编辑被拒绝以防止意外修改。
#[derive(Default)]
pub struct EditFileTool;

/// 单个编辑操作。
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EditOperation {
    old_text: String,
    new_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditFileArgs {
    path: PathBuf,
    /// 旧字符串（单个编辑时使用）。
    #[serde(default)]
    old_str: Option<String>,
    /// 新字符串（单个编辑时使用）。
    #[serde(default)]
    new_str: Option<String>,
    /// 设为 true 时替换所有匹配（而非要求唯一匹配），0 次匹配仍报错。
    #[serde(default)]
    replace_all: bool,
    /// 批量编辑：多个 oldText/newText 对，按顺序应用。
    #[serde(default)]
    edits: Option<Vec<EditOperation>>,
}

/// 将智能引号规范化为 ASCII 引号。
///
/// LLM 有时会生成智能引号（如 `""`、`''`），导致与代码中的 ASCII 引号不匹配。
/// 此函数将常见的智能引号字符替换为标准 ASCII 引号。
///
/// ## 替换规则
///
/// - `"` (U+201C) → `"` (U+0022)
/// - `"` (U+201D) → `"` (U+0022)
/// - `'` (U+2018) → `'` (U+0027)
/// - `'` (U+2019) → `'` (U+0027)
#[allow(clippy::collapsible_str_replace)]
fn normalize_quotes(s: &str) -> String {
    s.replace('\u{201C}', "\"")
        .replace('\u{201D}', "\"")
        .replace('\u{2018}', "'")
        .replace('\u{2019}', "'")
}

/// 在 haystack 中查找 needle 的唯一出现位置。
///
/// **为什么需要重叠检测**: 如果只按 `needle.len()` 步进，对于 `"ababa"` 中搜索 `"aba"`
/// 会漏掉位置 2 的重叠匹配。edit_file 要求 oldStr 在文件中必须唯一，
/// 因此需要逐 UTF-8 标量步进来捕获所有可能的匹配位置。
/// 找到多个匹配时返回错误，拒绝编辑以防止意外修改错误的位置。
fn find_unique_occurrence(haystack: &str, needle: &str) -> Option<Result<usize>> {
    if needle.is_empty() {
        return None;
    }

    let mut first_match = None;
    let mut offset = 0usize;
    while let Some(relative_pos) = haystack[offset..].find(needle) {
        let absolute_pos = offset + relative_pos;
        if first_match.replace(absolute_pos).is_some() {
            return Some(Err(AstrError::Validation(
                "oldStr appears multiple times, must be unique to edit safely".to_string(),
            )));
        }

        // 步进一个 UTF-8 标量以检测重叠匹配
        let step = haystack[absolute_pos..]
            .chars()
            .next()
            .map_or(1, |c| c.len_utf8());
        offset = absolute_pos + step;
    }

    first_match.map(Ok)
}

#[async_trait]
impl Tool for EditFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "editFile".to_string(),
            description: "Edit an existing file by replacing exact text. Each oldStr must appear \
                          exactly once (unless replaceAll=true). Prefer this over rewriting the \
                          whole file for small changes."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to edit (relative to the current working directory or absolute)."
                    },
                    "oldStr": {
                        "type": "string",
                        "description": "Exact text to replace — must match exactly once in the file, \
                                        including whitespace and newlines. If not found or duplicated, \
                                        the edit is rejected."
                    },
                    "newStr": {
                        "type": "string",
                        "description": "Replacement text. Use empty string to delete oldStr."
                    },
                    "replaceAll": {
                        "type": "boolean",
                        "description": "If true, replaces all occurrences. If false (default), \
                                        requires an exact single match."
                    },
                    "edits": {
                        "type": "array",
                        "description": "Batch edits: array of {oldText, newText} pairs, applied in order",
                        "items": {
                            "type": "object",
                            "properties": {
                                "oldText": { "type": "string" },
                                "newText": { "type": "string" }
                            },
                            "required": ["oldText", "newText"]
                        }
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "write", "edit"])
            .permission("filesystem.write")
            .side_effect(SideEffect::Local)
            .prompt(
                ToolPromptMetadata::new(
                    "Apply a narrow, safety-checked string replacement inside an existing file.",
                    "Use `editFile` when you know the exact old text and want a minimal change. \
                     It rejects ambiguous replacements, which makes it safer than rewriting a \
                     whole file for small edits. Prefer `apply_patch` instead when you need \
                     multi-file edits, multiple distant changes, or file creation/deletion. If \
                     the file was changed after the last `readFile`, `editFile` will stop and ask \
                     for a fresh reread first.",
                )
                .caveat(
                    "`oldStr` must match exactly once — including whitespace, newlines, trailing \
                     spaces, tabs, and line endings (`\\r\\n` vs `\\n`). If rejected, `readFile` \
                     the region first.",
                )
                .caveat(
                    "When this session has already observed the file, `editFile` also checks that \
                     the file has not changed on disk since that observation. If it did, call \
                     `readFile` again before editing.",
                )
                .example(
                    "Change one function body: { path: \"src/lib.rs\", oldStr: \"fn a() { old \
                     }\", newStr: \"fn a() { new }\" }",
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

        let args: EditFileArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for editFile", e))?;

        // 验证参数：要么提供 oldStr/newStr，要么提供 edits
        let edits = match (&args.old_str, &args.new_str, &args.edits) {
            (Some(old_str), Some(new_str), None) => {
                if old_str.is_empty() {
                    return Err(AstrError::Validation("oldStr cannot be empty".to_string()));
                }
                vec![EditOperation {
                    old_text: old_str.clone(),
                    new_text: new_str.clone(),
                }]
            },
            (None, None, Some(edits)) => {
                if edits.is_empty() {
                    return Err(AstrError::Validation(
                        "edits array cannot be empty".to_string(),
                    ));
                }
                for (i, edit) in edits.iter().enumerate() {
                    if edit.old_text.is_empty() {
                        return Err(AstrError::Validation(format!(
                            "edits[{}].oldText cannot be empty",
                            i
                        )));
                    }
                }
                edits.clone()
            },
            (Some(_), Some(_), Some(_)) => {
                return Err(AstrError::Validation(
                    "cannot specify both oldStr/newStr and edits - use one or the other"
                        .to_string(),
                ));
            },
            _ => {
                return Err(AstrError::Validation(
                    "must specify either oldStr/newStr or edits array".to_string(),
                ));
            },
        };

        // 引号规范化：将智能引号转换为 ASCII 引号
        let edits: Vec<EditOperation> = edits
            .into_iter()
            .map(|edit| EditOperation {
                old_text: normalize_quotes(&edit.old_text),
                new_text: normalize_quotes(&edit.new_text),
            })
            .collect();

        let started_at = Instant::now();
        let path = resolve_path(ctx, &args.path)?;
        ensure_not_canonical_session_plan_write_target(ctx, &path, "editFile")?;

        // UNC 路径检查：防止 Windows NTLM 凭据泄露
        if is_unc_path(&path) {
            return Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "editFile".to_string(),
                ok: false,
                output: String::new(),
                error: Some(format!(
                    "UNC paths are not supported for security reasons (potential NTLM credential \
                     leak on Windows). Path: '{}'",
                    path.display()
                )),
                metadata: Some(json!({
                    "path": path.to_string_lossy(),
                    "uncPath": true,
                })),
                continuation: None,
                duration_ms: started_at.elapsed().as_millis() as u64,
                truncated: false,
            });
        }

        // 符号链接检查：防止绕过路径沙箱
        if is_symlink(&path)? {
            return Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "editFile".to_string(),
                ok: false,
                output: String::new(),
                error: Some(format!(
                    "refusing to edit symlink '{}' (symlinks may point outside the intended \
                     target path)",
                    path.display()
                )),
                metadata: Some(json!({
                    "path": path.to_string_lossy(),
                    "isSymlink": true,
                })),
                continuation: None,
                duration_ms: started_at.elapsed().as_millis() as u64,
                truncated: false,
            });
        }

        if let Some(stale_result) = stale_file_guard_result(ctx, &path, &tool_call_id, started_at)?
        {
            return Ok(stale_result);
        }

        // 文件大小检查：防止编辑超大文件导致 OOM
        if path.exists() {
            let metadata = std::fs::metadata(&path).map_err(|e| {
                AstrError::io(
                    format!("failed reading metadata for '{}'", path.display()),
                    e,
                )
            })?;
            if metadata.len() > MAX_EDIT_FILE_SIZE {
                return Ok(ToolExecutionResult {
                    tool_call_id,
                    tool_name: "editFile".to_string(),
                    ok: false,
                    output: String::new(),
                    error: Some(format!(
                        "file too large to edit ({} bytes), maximum is {} bytes (1 GiB)",
                        metadata.len(),
                        MAX_EDIT_FILE_SIZE
                    )),
                    metadata: Some(json!({
                        "path": path.to_string_lossy(),
                        "bytes": metadata.len(),
                        "tooLarge": true,
                    })),
                    continuation: None,
                    duration_ms: started_at.elapsed().as_millis() as u64,
                    truncated: false,
                });
            }
        }

        let original_content = read_utf8_file(&path).await?;
        check_cancel(ctx.cancel())?;

        let mut content = original_content.clone();
        let mut total_edits = 0usize;

        for edit in &edits {
            check_cancel(ctx.cancel())?;

            content = if args.replace_all {
                // replace_all 模式：替换所有出现
                if !content.contains(&edit.old_text) {
                    return make_edit_error_result(
                        &tool_call_id,
                        &format!("oldText '{}' not found in file", edit.old_text),
                        &path,
                        started_at,
                    );
                }
                total_edits += content.matches(&edit.old_text).count();
                content.replace(&edit.old_text, &edit.new_text)
            } else {
                // 唯一匹配模式：必须恰好出现一次
                let match_start = match find_unique_occurrence(&content, &edit.old_text) {
                    Some(Ok(pos)) => pos,
                    Some(Err(_)) => {
                        return make_edit_error_result(
                            &tool_call_id,
                            &format!(
                                "oldText '{}' appears multiple times, must be unique to edit \
                                 safely",
                                edit.old_text
                            ),
                            &path,
                            started_at,
                        );
                    },
                    None => {
                        return make_edit_error_result(
                            &tool_call_id,
                            &format!("oldText '{}' not found in file", edit.old_text),
                            &path,
                            started_at,
                        );
                    },
                };

                let match_end = match_start + edit.old_text.len();
                let mut replaced = String::with_capacity(
                    content.len() - edit.old_text.len() + edit.new_text.len(),
                );
                replaced.push_str(&content[..match_start]);
                replaced.push_str(&edit.new_text);
                replaced.push_str(&content[match_end..]);
                total_edits += 1;
                replaced
            };
        }

        let report = build_text_change_report(&path, "updated", Some(&original_content), &content);
        check_cancel(ctx.cancel())?;
        write_text_file(&path, &content, false).await?;
        // 编辑成功后刷新观察快照，允许同一 session 在未发生外部改动时继续连续 edit。
        let observation = remember_file_observation(ctx, &path)?;

        let metadata = if edits.len() > 1 {
            json!({
                "path": path.to_string_lossy(),
                "editsApplied": edits.len(),
                "totalReplacements": total_edits,
                "contentFingerprint": observation.content_fingerprint,
                "modifiedUnixNanos": observation.modified_unix_nanos,
                "diff": report.metadata.get("diff").cloned().unwrap_or(json!(null)),
            })
        } else {
            let mut metadata = report.metadata;
            if let Some(object) = metadata.as_object_mut() {
                object.insert(
                    "contentFingerprint".to_string(),
                    json!(observation.content_fingerprint),
                );
                object.insert(
                    "modifiedUnixNanos".to_string(),
                    json!(observation.modified_unix_nanos),
                );
            }
            metadata
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "editFile".to_string(),
            ok: true,
            output: if edits.len() > 1 {
                format!("{} ({} edits applied)", report.summary, edits.len())
            } else {
                report.summary
            },
            error: None,
            metadata: Some(metadata),
            continuation: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

/// 在已有观察快照的前提下，拒绝对已被外部修改的文件直接编辑。
///
/// 这里不强制“所有 edit 都必须先 read”，因为首轮编辑可能已经拿到精确 oldStr。
/// 但一旦当前 session 之前观察过该文件，就要求磁盘版本仍然一致，避免 LLM
/// 基于过时内容继续写入。
fn stale_file_guard_result(
    ctx: &ToolContext,
    path: &Path,
    tool_call_id: &str,
    started_at: Instant,
) -> std::result::Result<Option<ToolExecutionResult>, AstrError> {
    let Some(previous_observation) = load_file_observation(ctx, path)? else {
        return Ok(None);
    };
    let current_observation = capture_file_observation(path)?;
    if file_observation_matches(&previous_observation, &current_observation) {
        return Ok(None);
    }

    make_edit_error_result(
        tool_call_id,
        &format!(
            "file changed on disk after the last read in this session. Call readFile on '{}' \
             first, then retry editFile.",
            path.display()
        ),
        path,
        started_at,
    )
    .map(Some)
}

/// 构建 editFile 失败时的统一响应。
fn make_edit_error_result(
    tool_call_id: &str,
    error: &str,
    path: &Path,
    started_at: Instant,
) -> std::result::Result<ToolExecutionResult, AstrError> {
    Ok(ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: "editFile".to_string(),
        ok: false,
        output: String::new(),
        error: Some(error.to_string()),
        metadata: Some(json!({
            "path": path.to_string_lossy(),
        })),
        continuation: None,
        duration_ms: started_at.elapsed().as_millis() as u64,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builtin_tools::read_file::ReadFileTool,
        test_support::{canonical_tool_path, test_tool_context_for},
    };

    #[tokio::test]
    async fn edit_file_replaces_unique_occurrence() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-ok".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "world world");
        assert_eq!(
            result.metadata.expect("metadata should exist")["path"],
            json!(canonical_tool_path(&file).to_string_lossy().to_string())
        );
    }

    #[tokio::test]
    async fn edit_file_refuses_when_old_str_missing() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-missing".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "missing",
                    "newStr": "world"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("not found"));
    }

    #[tokio::test]
    async fn edit_file_refuses_when_old_str_appears_multiple_times() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello hello")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-multi".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("multiple times"));
    }

    #[tokio::test]
    async fn edit_file_refuses_when_old_str_overlaps_itself() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "ababa")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-overlap".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "aba",
                    "newStr": "x"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("multiple times"));
    }

    #[tokio::test]
    async fn edit_file_returns_interrupted_error_when_cancelled() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;
        let cancel = {
            let ctx = test_tool_context_for(temp.path());
            ctx.cancel().cancel();
            ctx
        };

        let err = tool
            .execute(
                "tc-edit-cancel".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &cancel,
            )
            .await
            .expect_err("editFile should fail");

        assert!(err.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn edit_file_replace_all_replaces_every_occurrence() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "foo bar foo baz foo")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-replace-all".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "foo",
                    "newStr": "qux",
                    "replaceAll": true
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "qux bar qux baz qux");
    }

    #[tokio::test]
    async fn edit_file_replace_all_errors_when_no_match() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-replace-all-missing".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "missing",
                    "newStr": "x",
                    "replaceAll": true
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("not found"));
    }

    #[tokio::test]
    async fn edit_file_rejects_when_file_changed_after_read_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let ctx = test_tool_context_for(temp.path());
        let read_tool = ReadFileTool;
        let edit_tool = EditFileTool;

        let read_result = read_tool
            .execute(
                "tc-read-before-edit".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                }),
                &ctx,
            )
            .await
            .expect("readFile should execute");
        assert!(read_result.ok);

        // 模拟编辑器或其他进程在 LLM 之外改动了文件。
        tokio::fs::write(&file, "hello from editor")
            .await
            .expect("external write should work");

        let result = edit_tool
            .execute(
                "tc-edit-stale-after-read".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &ctx,
            )
            .await
            .expect("editFile should return a tool result");

        assert!(!result.ok);
        assert!(
            result
                .error
                .unwrap_or_default()
                .contains("Call readFile on")
        );
    }

    #[tokio::test]
    async fn edit_file_allows_observed_file_when_unchanged() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello world")
            .await
            .expect("seed write should work");
        let ctx = test_tool_context_for(temp.path());
        let read_tool = ReadFileTool;
        let edit_tool = EditFileTool;

        let read_result = read_tool
            .execute(
                "tc-read-fresh".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                }),
                &ctx,
            )
            .await
            .expect("readFile should execute");
        assert!(read_result.ok);

        let result = edit_tool
            .execute(
                "tc-edit-after-fresh-read".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world"
                }),
                &ctx,
            )
            .await
            .expect("editFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "world world");
    }

    #[tokio::test]
    async fn edit_file_refreshes_observation_after_successful_edit() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "alpha beta gamma")
            .await
            .expect("seed write should work");
        let ctx = test_tool_context_for(temp.path());
        let read_tool = ReadFileTool;
        let edit_tool = EditFileTool;

        let read_result = read_tool
            .execute(
                "tc-read-before-chain-edit".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                }),
                &ctx,
            )
            .await
            .expect("readFile should execute");
        assert!(read_result.ok);

        let first_edit = edit_tool
            .execute(
                "tc-first-edit".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "alpha",
                    "newStr": "delta"
                }),
                &ctx,
            )
            .await
            .expect("first edit should execute");
        assert!(first_edit.ok);

        let second_edit = edit_tool
            .execute(
                "tc-second-edit".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "gamma",
                    "newStr": "omega"
                }),
                &ctx,
            )
            .await
            .expect("second edit should execute");
        assert!(second_edit.ok);

        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "delta beta omega");
    }

    #[tokio::test]
    async fn edit_file_batch_edits_applied_in_order() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("code.rs");
        tokio::fs::write(&file, "fn old_name() { old_body }")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-batch".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "edits": [
                        {"oldText": "old_name", "newText": "new_name"},
                        {"oldText": "old_body", "newText": "new_body"}
                    ]
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("editFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&file)
            .await
            .expect("file should be readable");
        assert_eq!(content, "fn new_name() { new_body }");
        // 验证 metadata 包含编辑数量
        let meta = result.metadata.expect("metadata should exist");
        assert_eq!(meta["editsApplied"], json!(2));
    }

    #[tokio::test]
    async fn edit_file_allows_relative_path_outside_working_dir() {
        let parent = tempfile::tempdir().expect("tempdir should be created");
        let workspace = parent.path().join("workspace");
        let outside = parent.path().join("outside.txt");
        tokio::fs::create_dir_all(&workspace)
            .await
            .expect("workspace should be created");
        tokio::fs::write(&outside, "alpha beta")
            .await
            .expect("outside file should be written");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-outside".to_string(),
                json!({
                    "path": "../outside.txt",
                    "oldStr": "alpha",
                    "newStr": "omega"
                }),
                &test_tool_context_for(&workspace),
            )
            .await
            .expect("editFile should execute");

        assert!(result.ok);
        let content = tokio::fs::read_to_string(&outside)
            .await
            .expect("outside file should be readable");
        assert_eq!(content, "omega beta");
    }

    #[tokio::test]
    async fn edit_file_batch_edits_rejects_empty_array() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-empty-batch".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "edits": []
                }),
                &test_tool_context_for(temp.path()),
            )
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn edit_file_rejects_canonical_session_plan_targets() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp
            .path()
            .join(".astrcode-test-state")
            .join("sessions")
            .join("session-test")
            .join("plan")
            .join("cleanup-crates.md");
        tokio::fs::create_dir_all(file.parent().expect("plan file should have a parent"))
            .await
            .expect("plan dir should be created");
        tokio::fs::write(&file, "# Plan: Cleanup crates\n")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-plan".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "Cleanup crates",
                    "newStr": "Prompt governance"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("upsertSessionPlan")
        );
    }

    #[tokio::test]
    async fn edit_file_cannot_mix_single_and_batch() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("hello.txt");
        tokio::fs::write(&file, "hello")
            .await
            .expect("seed write should work");
        let tool = EditFileTool;

        let result = tool
            .execute(
                "tc-edit-mixed".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "oldStr": "hello",
                    "newStr": "world",
                    "edits": [{"oldText": "a", "newText": "b"}]
                }),
                &test_tool_context_for(temp.path()),
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot specify both")
        );
    }
}
