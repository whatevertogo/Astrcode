//! # ReadFile 工具
//!
//! 实现 `readFile` 工具，用于读取文件内容。
//!
//! ## 设计要点
//!
//! - **文本文件**: 读取 UTF-8 文本，支持行号、偏移、截断
//! - **图片文件**: 返回 base64 编码和 media type，供多模态模型使用
//! - **PDF 文件**: 读取并返回内容（需要 pdf_extract 特性）
//! - 默认最大返回 20,000 字符（context window 友好值）
//! - 截断点位于 UTF-8 字符边界
//! - 检测二进制文件并返回友好错误提示

use std::{
    fs,
    io::{BufRead, BufReader, Read as _},
    path::PathBuf,
    time::Instant,
};

use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Deserialize;
use serde_json::json;

use crate::builtin_tools::fs_common::{check_cancel, resolve_read_path};

/// 二进制检测采样大小（前 N 字节）。
const BINARY_DETECT_SAMPLE_SIZE: usize = 8192;

/// 图片文件最大大小（20MB），超过此大小的图片拒绝读取。
const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024;

/// 支持的图片扩展名及其 MIME 类型。
///
/// TODO: Decide whether SVG should keep following the multimodal-image path or fall back to the
/// plain-text reader. SVG is text, so code-oriented workflows often want grep/read semantics
/// instead of base64 transport.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
    ("ico", "image/x-icon"),
    ("bmp", "image/bmp"),
];

/// ReadFile 工具实现。
///
/// 读取 UTF-8 文本文件，支持按行偏移和字符预算。
#[derive(Default)]
pub struct ReadFileTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFileArgs {
    path: PathBuf,
    /// 最大返回字符数，默认 20,000。
    #[serde(default)]
    max_chars: Option<usize>,
    /// 起始行号（0-based），用于跳过文件头部。
    #[serde(default)]
    offset: Option<usize>,
    /// 最多返回的行数，与 offset 配合使用。
    #[serde(default)]
    limit: Option<usize>,
    /// 是否在每行前显示行号（默认 true），对 LLM 定位代码效率更高。
    #[serde(default = "default_true")]
    line_numbers: bool,
}

/// 根据文件扩展名获取图片的 MIME 类型。
fn get_image_mime_type(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    IMAGE_TYPES
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, mime)| *mime)
}

/// 检查文件是否为图片。
fn is_image_file(path: &std::path::Path) -> bool {
    get_image_mime_type(path).is_some()
}

/// 读取图片文件并返回 base64 编码。
fn read_image_file(
    path: &std::path::Path,
    max_inline_bytes: usize,
) -> Result<(String, String, usize)> {
    let metadata = fs::metadata(path).map_err(|e| {
        AstrError::io(
            format!("failed reading metadata for '{}'", path.display()),
            e,
        )
    })?;
    let file_size = metadata.len() as usize;

    if file_size > MAX_IMAGE_SIZE {
        return Err(AstrError::Validation(format!(
            "image file too large ({} bytes), maximum allowed is {} bytes",
            file_size, MAX_IMAGE_SIZE
        )));
    }

    // The current tool transport only persists final output as UTF-8 strings inside storage
    // events. Refusing oversize image payloads here avoids exploding JSONL/SSE traffic until we
    // have a dedicated binary/blob channel for multimodal artifacts.
    let estimated_base64_bytes = file_size.div_ceil(3) * 4;
    if estimated_base64_bytes > max_inline_bytes {
        return Err(AstrError::Validation(format!(
            "image payload would expand to about {} bytes after base64 encoding, exceeding the \
             current inline limit of {} bytes",
            estimated_base64_bytes, max_inline_bytes
        )));
    }

    let content = fs::read(path)
        .map_err(|e| AstrError::io(format!("failed reading image '{}'", path.display()), e))?;
    let mime_type = get_image_mime_type(path).unwrap_or("application/octet-stream");
    let base64_data = BASE64.encode(&content);

    Ok((base64_data, mime_type.to_string(), file_size))
}

fn default_true() -> bool {
    true
}

/// 检测文件是否为二进制文件。
///
/// 读取文件前 `BINARY_DETECT_SAMPLE_SIZE` 字节，检测是否包含 NUL 字节。
/// NUL 字节是文本文件几乎不可能出现的可靠二进制指标。
fn is_binary_file(path: &std::path::Path) -> Result<bool> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        AstrError::io(
            format!("failed reading metadata for '{}'", path.display()),
            e,
        )
    })?;
    let file_size = metadata.len() as usize;
    let sample_size = BINARY_DETECT_SAMPLE_SIZE.min(file_size);

    let mut file = fs::File::open(path)
        .map_err(|e| AstrError::io(format!("failed opening file '{}'", path.display()), e))?;
    let mut sample = vec![0u8; sample_size];
    let bytes_read = file
        .read(&mut sample)
        .map_err(|e| AstrError::io("failed reading file for binary detection", e))?;
    sample.truncate(bytes_read);
    // NUL 字节是文本文件中几乎不可能出现的可靠二进制指标
    Ok(sample.contains(&0))
}

#[async_trait]
impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "readFile".to_string(),
            description: "Read a file's contents. Supports text files, images (returns base64), \
                          and respects line-based offset/limit for targeted reads."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
                    },
                    "maxChars": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum characters to return (default 20000)"
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Starting line number (0-based). Skips lines before this offset."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum number of lines to read from the offset."
                    },
                    "lineNumbers": {
                        "type": "boolean",
                        "description": "Prepend line numbers to each line (default true)"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "read"])
            .permission("filesystem.read")
            .side_effect(SideEffectLevel::None)
            .concurrency_safe(true)
            .compact_clearable(true)
            .prompt(
                ToolPromptMetadata::new(
                    "Read file contents — supports text, images (base64), targeted line-range \
                     reads.",
                    "Use after `grep`/`findFiles` gives you a path. Set `offset` (0-based line) + \
                     `limit` to read a specific range. Set `lineNumbers: false` to skip \
                     line-number prefixes.",
                )
                .caveat(
                    "Output is capped at `maxChars` (default 20000). If truncated, use `offset` + \
                     `limit` to read the next chunk.",
                )
                .example("Read lines 50–100: { path: \"src/main.rs\", offset: 50, limit: 50 }")
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

        let args: ReadFileArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for readFile", e))?;
        let started_at = Instant::now();
        let path = resolve_read_path(ctx, &args.path)?;

        // 图片文件处理：返回 base64 编码
        if is_image_file(&path) {
            return match read_image_file(&path, ctx.max_output_size()) {
                Ok((base64_data, mime_type, file_size)) => Ok(ToolExecutionResult {
                    tool_call_id,
                    tool_name: "readFile".to_string(),
                    ok: true,
                    output: json!({
                        "type": "image",
                        "mediaType": mime_type,
                        "data": base64_data,
                    })
                    .to_string(),
                    error: None,
                    metadata: Some(json!({
                        "path": path.to_string_lossy(),
                        "bytes": file_size,
                        "fileType": "image",
                    })),
                    duration_ms: started_at.elapsed().as_millis() as u64,
                    truncated: false,
                }),
                Err(e) => Ok(ToolExecutionResult {
                    tool_call_id,
                    tool_name: "readFile".to_string(),
                    ok: false,
                    output: String::new(),
                    error: Some(e.to_string()),
                    metadata: Some(json!({
                        "path": path.to_string_lossy(),
                    })),
                    duration_ms: started_at.elapsed().as_millis() as u64,
                    truncated: false,
                }),
            };
        }

        let max_chars = args.max_chars.unwrap_or(20_000);

        // 二进制文件检测：避免将二进制文件内容作为乱码返回，浪费 context window
        if is_binary_file(&path)? {
            let metadata = std::fs::metadata(&path).ok();
            let file_size = metadata.map(|m| m.len() as usize).unwrap_or(0);
            return Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "readFile".to_string(),
                ok: false,
                output: String::new(),
                error: Some(format!(
                    "file appears to be binary ({} bytes). Use the shell tool with 'xxd' or \
                     'file' command to inspect it.",
                    file_size
                )),
                metadata: Some(json!({
                    "path": path.to_string_lossy(),
                    "bytes": file_size,
                    "binary": true,
                })),
                duration_ms: started_at.elapsed().as_millis() as u64,
                truncated: false,
            });
        }

        let file = fs::File::open(&path)
            .map_err(|e| AstrError::io(format!("failed opening file '{}'", path.display()), e))?;
        let total_bytes = file
            .metadata()
            .map_err(|e| {
                AstrError::io(
                    format!("failed reading metadata for '{}'", path.display()),
                    e,
                )
            })?
            .len() as usize;
        let reader = BufReader::new(file);

        // TODO: Replace this second full-file pass with a lazy/streaming width strategy once
        // line-number formatting rules settle. The current implementation is simple and stable,
        // but it doubles I/O for very large text files.
        // 需要先知道总行数来计算行号宽度
        let total_line_count = count_total_lines(&path, ctx.cancel())?;

        let (text, _returned_lines, truncated) = if args.offset.is_some() || args.limit.is_some() {
            read_lines_range(
                reader,
                args.offset.unwrap_or(0),
                args.limit,
                max_chars,
                args.line_numbers,
                total_line_count,
                ctx.cancel(),
            )
        } else {
            read_file_full(
                reader,
                max_chars,
                args.line_numbers,
                total_line_count,
                ctx.cancel(),
            )
        }?;

        let meta = if args.offset.is_some() || args.limit.is_some() {
            json!({
                "path": path.to_string_lossy(),
                "bytes": total_bytes,
                "total_lines": total_line_count,
                "offset": args.offset.unwrap_or(0),
                "limit": args.limit,
                "truncated": truncated,
            })
        } else {
            json!({
                "path": path.to_string_lossy(),
                "bytes": total_bytes,
                "truncated": truncated,
            })
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "readFile".to_string(),
            ok: true,
            output: text,
            error: None,
            metadata: Some(meta),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

/// 统计文件总行数（用于计算行号显示宽度)。
fn count_total_lines(path: &std::path::Path, cancel: &astrcode_core::CancelToken) -> Result<usize> {
    let file = fs::File::open(path)
        .map_err(|e| AstrError::io(format!("failed opening file '{}'", path.display()), e))?;
    let reader = BufReader::new(file);
    let mut count = 0usize;
    for line_result in reader.lines() {
        check_cancel(cancel)?;
        let _ = line_result.map_err(|e| AstrError::io("failed counting lines", e))?;
        count += 1;
    }
    Ok(count)
}

/// 讣算行号的显示宽度（字符数)。
///
/// 例如 999 行需要 3 位宽度， 1000 行需要 4 位宽度。
fn line_number_width(total_lines: usize) -> usize {
    if total_lines == 0 {
        return 1;
    }
    let digits = format!("{}", total_lines).len();
    // 至少 4 位,保持对齐美观
    digits.max(4)
}

/// 格式化带行号的一行。
fn format_line(number: usize, content: &str, width: usize) -> String {
    format!("{number:>width$}\t{content}")
}

/// 读取文件的前 max_chars 个字符。
fn read_file_full(
    reader: BufReader<fs::File>,
    max_chars: usize,
    line_numbers: bool,
    total_lines: usize,
    cancel: &astrcode_core::CancelToken,
) -> Result<(String, usize, bool)> {
    let num_width = if line_numbers {
        line_number_width(total_lines)
    } else {
        0
    };
    let mut output = String::new();
    let mut line_no = 0usize;

    for line_result in reader.lines() {
        check_cancel(cancel)?;
        let line = line_result.map_err(|e| AstrError::io("failed reading file line", e))?;
        line_no += 1;

        let formatted = if line_numbers {
            format_line(line_no, &line, num_width)
        } else {
            line.clone()
        };

        let remaining = max_chars.saturating_sub(output.chars().count());
        // 已超出字符预算，后续内容全部截断
        if remaining == 0 {
            return Ok((output, line_no, true));
        }

        if !output.is_empty() {
            output.push('\n');
        }

        let formatted_chars = formatted.chars().count();
        if formatted_chars <= remaining {
            output.push_str(&formatted);
        } else {
            // 当前行需要截断：按字符数计算安全的字节边界
            let boundary = char_count_to_byte_offset(&formatted, remaining);
            output.push_str(&formatted[..boundary]);
            return Ok((output, line_no, true));
        }
    }

    Ok((output, line_no, false))
}

/// 将字符数量转换为字节偏移量。
///
/// `floor_char_boundary(n)` 的参数是字节位置而非字符数量，
/// 因此不能直接用于"取前 N 个字符"的场景。
fn char_count_to_byte_offset(s: &str, char_count: usize) -> usize {
    s.char_indices()
        .nth(char_count)
        .map_or(s.len(), |(idx, _)| idx)
}

/// 按行范围读取：跳过 offset 行，最多读取 limit 行。
///
/// 返回 `(output, total_line_count, truncated)`，其中 `total_line_count`
/// 是文件的实际总行数（即使超出 limit 也会继续计数)。
fn read_lines_range(
    reader: BufReader<fs::File>,
    offset: usize,
    limit: Option<usize>,
    max_chars: usize,
    line_numbers: bool,
    total_lines: usize,
    cancel: &astrcode_core::CancelToken,
) -> Result<(String, usize, bool)> {
    let num_width = if line_numbers {
        line_number_width(total_lines)
    } else {
        0
    };
    let mut output = String::new();
    let mut line_count = 0usize;
    let mut lines_read = 0usize;
    let max_lines = limit.unwrap_or(usize::MAX);

    for line_result in reader.lines() {
        check_cancel(cancel)?;
        let line = line_result.map_err(|e| AstrError::io("failed reading file line", e))?;
        line_count += 1;

        if line_count <= offset {
            continue;
        }

        // 已读够 limit 行，跳过但继续计数以获取准确总行数
        if lines_read >= max_lines {
            continue;
        }

        // line_count 是 1-based 行号（用于显示）
        let formatted = if line_numbers {
            format_line(line_count, &line, num_width)
        } else {
            line.to_string()
        };

        let remaining = max_chars.saturating_sub(output.chars().count());
        if remaining == 0 {
            return Ok((output, line_count, true));
        }
        if !output.is_empty() {
            output.push('\n');
        }

        let take = remaining.min(formatted.chars().count());
        if take == 0 && line_numbers {
            // 行号本身就超出预算
            return Ok((output, line_count, true));
        }
        output.push_str(&formatted[..char_count_to_byte_offset(&formatted, take)]);
        lines_read += 1;
        // 单行超出字符预算
        if take < formatted.chars().count() {
            return Ok((output, line_count, true));
        }
    }

    // 自然 EOF：只有字符预算耗尽才算截断
    let truncated = output.chars().count() >= max_chars && line_count > offset;
    Ok((output, line_count, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

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
                json!({ "path": file.to_string_lossy(), "maxChars": 3, "lineNumbers": false }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert_eq!(result.output, "abc");
        let metadata = result.metadata.expect("metadata should exist");
        assert_eq!(metadata["bytes"], json!(6));
        assert_eq!(metadata["truncated"], json!(true));
    }

    #[tokio::test]
    async fn read_file_tool_truncates_at_utf8_char_boundary() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "你好a")
            .await
            .expect("write should work");
        let tool = ReadFileTool;
        let result = tool
            .execute(
                "tc4".to_string(),
                json!({ "path": file.to_string_lossy(), "maxChars": 1, "lineNumbers": false }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert_eq!(result.output, "你");
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn read_file_supports_offset_and_limit() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "line0\nline1\nline2\nline3\nline4\n")
            .await
            .expect("write should work");
        let tool = ReadFileTool;

        let result = tool
            .execute(
                "tc-offset".to_string(),
                json!({ "path": file.to_string_lossy(), "offset": 2, "limit": 2 }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        // 默认带行号，所以输出格式是 "行号\t内容"
        assert!(!result.truncated);
        let meta = result.metadata.expect("metadata should exist");
        assert_eq!(meta["total_lines"], json!(5));
        assert_eq!(meta["limit"], json!(2));
        // 验证输出包含行号 3 和 4
        assert!(result.output.contains("3\tline2"));
        assert!(result.output.contains("4\tline3"));
    }

    #[tokio::test]
    async fn read_file_offset_metadata_keeps_real_total_lines_when_truncated() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "line0\nline1\nline2\nline3\nline4\n")
            .await
            .expect("write should work");
        let tool = ReadFileTool;

        let result = tool
            .execute(
                "tc-offset-truncated".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "offset": 1,
                    "limit": 10,
                    "maxChars": 6,
                    "lineNumbers": false
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert!(result.truncated);
        let meta = result.metadata.expect("metadata should exist");
        assert_eq!(meta["total_lines"], json!(5));
    }

    #[tokio::test]
    async fn read_file_line_numbers_disabled() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("sample.txt");
        tokio::fs::write(&file, "line0\nline1\nline2\n")
            .await
            .expect("write should work");
        let tool = ReadFileTool;

        let result = tool
            .execute(
                "tc-no-lnum".to_string(),
                json!({
                    "path": file.to_string_lossy(),
                    "lineNumbers": false
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        // 关闭行号后输出应为纯文本
        assert_eq!(result.output, "line0\nline1\nline2");
    }

    #[tokio::test]
    async fn read_file_detects_binary() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("binary.bin");
        // 写入包含 NUL 字节的二进制数据
        tokio::fs::write(&file, b"hello\x00world")
            .await
            .expect("write should work");
        let tool = ReadFileTool;
        let result = tool
            .execute(
                "tc-binary".to_string(),
                json!({ "path": file.to_string_lossy() }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert!(!result.ok);
        assert!(result.error.unwrap_or_default().contains("binary"));
    }

    #[tokio::test]
    async fn read_file_empty_file_not_detected_as_binary() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("empty.txt");
        tokio::fs::write(&file, "")
            .await
            .expect("write should work");
        let tool = ReadFileTool;
        let result = tool
            .execute(
                "tc-empty".to_string(),
                json!({ "path": file.to_string_lossy() }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        // 空文件不包含 NUL 字节,不应被检测为二进制
        assert!(result.ok);
        assert!(result.output.is_empty());
    }

    #[tokio::test]
    async fn read_file_returns_inline_image_payload_for_small_images() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("pixel.png");
        tokio::fs::write(&file, [0x89, b'P', b'N', b'G'])
            .await
            .expect("write should work");
        let tool = ReadFileTool;

        let result = tool
            .execute(
                "tc-image-inline".to_string(),
                json!({ "path": file.to_string_lossy() }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should succeed");

        assert!(result.ok);
        let payload: serde_json::Value =
            serde_json::from_str(&result.output).expect("image output should stay JSON");
        assert_eq!(payload["type"], json!("image"));
        assert_eq!(payload["mediaType"], json!("image/png"));
        assert!(
            payload["data"]
                .as_str()
                .is_some_and(|data| !data.is_empty())
        );
    }

    #[tokio::test]
    async fn read_file_rejects_images_that_do_not_fit_inline_transport() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let file = temp.path().join("large.png");
        tokio::fs::write(&file, vec![0u8; 800_000])
            .await
            .expect("write should work");
        let tool = ReadFileTool;

        let result = tool
            .execute(
                "tc-image-too-large".to_string(),
                json!({ "path": file.to_string_lossy() }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("readFile should return a tool result");

        assert!(!result.ok);
        assert!(
            result
                .error
                .unwrap_or_default()
                .contains("exceeding the current inline limit"),
        );
    }
}
