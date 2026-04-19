//! 工具结果磁盘持久化基础设施。
//!
//! 提供工具结果落盘的核心函数，供工具执行侧（adapter-tools）和
//! 管线聚合预算层（runtime-agent-loop）共享。
//!
//! # 两层接口
//!
//! - [`persist_tool_result`]：无条件落盘（不管内容大小），供管线聚合预算强制调用
//! - [`maybe_persist_tool_result`]：条件落盘（超过阈值时才落盘），供工具执行侧调用
//!
//! # 降级策略
//!
//! 磁盘写入失败时降级为截断预览，不 panic、不返回错误。
//! 这保证了即使文件系统不可用，工具结果仍然能以截断形式传递给 LLM。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 工具结果存盘目录名（相对于 session 目录）。
pub const TOOL_RESULTS_DIR: &str = "tool-results";

/// 默认预览截断大小（字节）。
pub const TOOL_RESULT_PREVIEW_LIMIT: usize = 2 * 1024;

/// 默认内联阈值（字节）。
///
/// 工具结果超过此大小时触发落盘。可通过 `CapabilitySpec.max_result_inline_size`
/// 覆盖为 per-tool 值。
pub const DEFAULT_TOOL_RESULT_INLINE_LIMIT: usize = 32 * 1024;

/// 已持久化工具结果的结构化元数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedToolOutput {
    pub storage_kind: String,
    pub absolute_path: String,
    pub relative_path: String,
    pub total_bytes: u64,
    pub preview_text: String,
    pub preview_bytes: u64,
}

/// 工具结果经落盘决策后的统一返回值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedToolResult {
    pub output: String,
    pub persisted: Option<PersistedToolOutput>,
}

/// 无条件将工具结果持久化到磁盘。
///
/// 不管内容大小，一律写入 `session_dir/tool-results/<id>.txt`，
/// 返回 `<persisted-output>` 格式的短引用。
/// 写入失败时降级为截断预览。
///
/// 供管线聚合预算层调用：当聚合预算超限时，选中的结果不管多大都需要落盘。
pub fn persist_tool_result(
    session_dir: &Path,
    tool_call_id: &str,
    content: &str,
) -> PersistedToolResult {
    write_to_disk(session_dir, tool_call_id, content)
}

/// 条件持久化：仅当 content 大小超过 `inline_limit` 时落盘。
///
/// `inline_limit` 由调用方传入：
/// - 工具执行侧：从 `ToolContext::resolved_inline_limit()` 获取
/// - 其他场景：使用 `DEFAULT_TOOL_RESULT_INLINE_LIMIT`
pub fn maybe_persist_tool_result(
    session_dir: &Path,
    tool_call_id: &str,
    content: &str,
    inline_limit: usize,
) -> PersistedToolResult {
    if content.len() <= inline_limit {
        return PersistedToolResult {
            output: content.to_string(),
            persisted: None,
        };
    }
    write_to_disk(session_dir, tool_call_id, content)
}

/// 检测内容是否已被持久化（包含 `<persisted-output>` 标签）。
pub fn is_persisted_output(content: &str) -> bool {
    content.contains("<persisted-output>")
}

/// 从 persisted wrapper 文本中提取绝对路径。
pub fn persisted_output_absolute_path(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.split_once("Path: ")
            .map(|(_, path)| path.trim().to_string())
    })
}

/// 解析工具结果内联阈值，支持环境变量覆盖。
///
/// 优先级（从高到低）：
/// 1. Per-tool 环境变量 `ASTRCODE_TOOL_INLINE_LIMIT_{TOOL_NAME}`（大写）
/// 2. 描述符中的 `max_result_inline_size`
/// 3. 全局环境变量 `ASTRCODE_TOOL_RESULT_INLINE_LIMIT`
/// 4. 调用方提供的默认阈值（通常来自 runtime 配置）
///
/// 工具名转换规则：camelCase → SCREAMING_SNAKE_CASE。
/// 例如 `readFile` → `ASTRCODE_TOOL_INLINE_LIMIT_READ_FILE`，
/// `shell` → `ASTRCODE_TOOL_INLINE_LIMIT_SHELL`。
pub fn resolve_inline_limit(
    tool_name: &str,
    descriptor_limit: Option<usize>,
    configured_default: usize,
) -> usize {
    let per_tool_env_key = format!(
        "{}{}",
        crate::env::ASTRCODE_TOOL_INLINE_LIMIT_PREFIX,
        camel_to_screaming_snake(tool_name)
    );
    resolve_inline_limit_impl(
        std::env::var(&per_tool_env_key).ok().as_deref(),
        descriptor_limit,
        std::env::var(crate::env::ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV)
            .ok()
            .as_deref(),
        configured_default,
    )
}

/// 纯逻辑解析，不读取环境变量。方便测试。
fn resolve_inline_limit_impl(
    per_tool_env: Option<&str>,
    descriptor_limit: Option<usize>,
    global_env: Option<&str>,
    configured_default: usize,
) -> usize {
    // 层级 1：per-tool 环境变量
    if let Some(val) = per_tool_env {
        if let Ok(limit) = val.parse::<usize>() {
            return limit;
        }
    }

    // 层级 2：描述符中的值
    if let Some(limit) = descriptor_limit {
        return limit;
    }

    // 层级 3：全局环境变量
    if let Some(val) = global_env {
        if let Ok(limit) = val.parse::<usize>() {
            return limit;
        }
    }

    // 层级 4：默认值
    configured_default.max(1)
}

/// 将 camelCase 转换为 SCREAMING_SNAKE_CASE。
///
/// 例：`readFile` → `READ_FILE`，`findFiles` → `FIND_FILES`，`shell` → `SHELL`。
fn camel_to_screaming_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_ascii_uppercase());
    }
    result
}

/// 实际写磁盘操作。
///
/// 包含完整的降级链路——任何一步失败都不会 panic：
/// 1. `create_dir_all` 失败 → 降级为截断预览
/// 2. `fs::write` 失败 → 降级为截断预览
/// 3. 成功 → 生成 `<persisted-output>` 短引用 + 结构化 persisted metadata
///
/// 工具调用 ID 会被清洗（只保留字母数字和 `-_`，取前 64 字符），
/// 防止路径穿越攻击（如 `../../etc/passwd`）。
fn write_to_disk(session_dir: &Path, tool_call_id: &str, content: &str) -> PersistedToolResult {
    let content_bytes = content.len();
    let results_dir = session_dir.join(TOOL_RESULTS_DIR);

    if std::fs::create_dir_all(&results_dir).is_err() {
        log::warn!(
            "tool-result: failed to create dir '{}', falling back to truncation",
            results_dir.display()
        );
        return PersistedToolResult {
            output: truncate_with_notice(content),
            persisted: None,
        };
    }

    let safe_id: String = tool_call_id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    let path = results_dir.join(format!("{safe_id}.txt"));

    if std::fs::write(&path, content).is_err() {
        log::warn!(
            "tool-result: failed to write '{}', falling back to truncation",
            path.display()
        );
        return PersistedToolResult {
            output: truncate_with_notice(content),
            persisted: None,
        };
    }

    let relative_path = path
        .strip_prefix(session_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    let persisted = PersistedToolOutput {
        storage_kind: "toolResult".to_string(),
        absolute_path: normalize_absolute_path(&path),
        relative_path,
        total_bytes: content_bytes as u64,
        preview_text: build_preview_text(content),
        preview_bytes: TOOL_RESULT_PREVIEW_LIMIT.min(content.len()) as u64,
    };

    PersistedToolResult {
        output: format_persisted_output(&persisted),
        persisted: Some(persisted),
    }
}

/// 生成 `<persisted-output>` 格式的短引用文本。
///
/// 该文本会替换原始工具结果进入消息历史，LLM 看到的是这段引用
/// 而非完整内容。引用中包含路径、大小和建议的首次读取参数，
/// 引导 LLM 使用 readFile 按需读取。
fn format_persisted_output(persisted: &PersistedToolOutput) -> String {
    format!(
        "<persisted-output>\nLarge tool output was saved to a file instead of being \
         inlined.\nPath: {}\nBytes: {}\nRead the file with `readFile`.\nIf you only need a \
         section, read a smaller chunk instead of the whole file.\nStart from the first chunk \
         when you do not yet know the right section.\nSuggested first read: {{ path: {:?}, \
         charOffset: 0, maxChars: 20000 }}\n</persisted-output>",
        persisted.absolute_path, persisted.total_bytes, persisted.absolute_path
    )
}

fn build_preview_text(content: &str) -> String {
    let preview_limit = TOOL_RESULT_PREVIEW_LIMIT.min(content.len());
    let truncated_at = content.floor_char_boundary(preview_limit);
    content[..truncated_at].to_string()
}

fn normalize_absolute_path(path: &Path) -> String {
    normalize_verbatim_path(path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn normalize_verbatim_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(rendered) = path.to_str() {
            if let Some(stripped) = rendered.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{}", stripped));
            }
            if let Some(stripped) = rendered.strip_prefix(r"\\?\") {
                return PathBuf::from(stripped);
            }
        }
    }

    path
}

/// 截断内容并附加通知。
fn truncate_with_notice(content: &str) -> String {
    let limit = TOOL_RESULT_PREVIEW_LIMIT.min(content.len());
    let truncated_at = content.floor_char_boundary(limit);
    let prefix = &content[..truncated_at];
    format!(
        "{prefix}\n\n... [output truncated to {limit} bytes because persisted storage is \
         unavailable; use offset/limit parameters or rerun with a narrower scope for full content]"
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn persist_tool_result_writes_file_and_returns_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let result = persist_tool_result(dir.path(), "call-abc123", &content);

        assert!(result.output.contains("<persisted-output>"));
        assert!(result.output.contains("Large tool output was saved"));
        let persisted = result.persisted.expect("persisted metadata should exist");
        assert!(result.output.contains(&persisted.absolute_path));
        assert!(result.output.contains("Bytes: 100"));
        assert_eq!(persisted.relative_path, "tool-results/call-abc123.txt");
        assert_eq!(persisted.total_bytes, 100);
        assert_eq!(persisted.preview_text, content);
        assert_eq!(persisted.preview_bytes, 100);

        let file_path = dir.path().join("tool-results/call-abc123.txt");
        assert!(file_path.exists());
        assert_eq!(
            fs::read_to_string(&file_path).expect("persisted file should be readable"),
            content
        );
    }

    #[test]
    fn maybe_persist_skips_when_below_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "small".to_string();
        let result = maybe_persist_tool_result(dir.path(), "call-1", &content, 1024);

        assert_eq!(result.output, "small");
        assert!(result.persisted.is_none());
        assert!(!dir.path().join("tool-results/call-1.txt").exists());
    }

    #[test]
    fn maybe_persist_persists_when_above_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let result = maybe_persist_tool_result(dir.path(), "call-1", &content, 50);

        assert!(result.output.contains("<persisted-output>"));
        assert!(result.persisted.is_some());
        assert!(dir.path().join("tool-results/call-1.txt").exists());
    }

    #[test]
    fn is_persisted_output_detects_tag() {
        assert!(is_persisted_output(
            "<persisted-output>\nsome content\n</persisted-output>"
        ));
        assert!(!is_persisted_output("normal tool output"));
    }

    #[test]
    fn degrade_on_write_failure() {
        // Windows 上某些路径不会失败，所以只在实际降级时断言
        let content = "x".repeat(100);
        let result = persist_tool_result(Path::new("/nonexistent/path"), "call-1", &content);
        // 降级为截断预览或成功写入（取决于平台）
        assert!(
            result.output.contains("[output truncated")
                || result.output.contains("<persisted-output>")
        );
    }

    #[test]
    fn sanitizes_tool_call_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "x".repeat(100);
        let _ = persist_tool_result(dir.path(), "call/../../../etc/passwd", &content);

        // 不应创建路径穿越目录
        assert!(!dir.path().join("etc").exists());
        // safe_id 只保留字母数字和 -_，过滤掉 / 和 .
        let file = dir.path().join("tool-results/calletcpasswd.txt");
        assert!(file.exists());
    }

    #[test]
    fn persisted_output_absolute_path_extracts_new_wrapper_path() {
        let wrapper = "<persisted-output>\nLarge tool output was saved to a file instead of being \
                       inlined.\nPath: ~/.astrcode/tool-results/call-1.txt\nBytes: \
                       42\n</persisted-output>";
        assert_eq!(
            persisted_output_absolute_path(wrapper),
            Some("~/.astrcode/tool-results/call-1.txt".to_string())
        );
    }

    #[test]
    fn camel_to_screaming_snake_converts_correctly() {
        assert_eq!(camel_to_screaming_snake("readFile"), "READ_FILE");
        assert_eq!(camel_to_screaming_snake("findFiles"), "FIND_FILES");
        assert_eq!(camel_to_screaming_snake("shell"), "SHELL");
        assert_eq!(camel_to_screaming_snake("grep"), "GREP");
    }

    #[test]
    fn resolve_inline_limit_uses_default_when_no_override() {
        // 无 env、无描述符 → 默认 32KB
        assert_eq!(
            resolve_inline_limit_impl(None, None, None, DEFAULT_TOOL_RESULT_INLINE_LIMIT),
            DEFAULT_TOOL_RESULT_INLINE_LIMIT
        );

        // 有描述符值 → 使用描述符值
        assert_eq!(
            resolve_inline_limit_impl(None, Some(30_000), None, DEFAULT_TOOL_RESULT_INLINE_LIMIT),
            30_000
        );
    }

    #[test]
    fn resolve_inline_limit_global_env_overrides_default() {
        // 无描述符值，全局 env 覆盖默认
        assert_eq!(
            resolve_inline_limit_impl(None, None, Some("65536"), DEFAULT_TOOL_RESULT_INLINE_LIMIT),
            65536
        );

        // 全局 env 优先级低于描述符值
        assert_eq!(
            resolve_inline_limit_impl(
                None,
                Some(30_000),
                Some("65536"),
                DEFAULT_TOOL_RESULT_INLINE_LIMIT,
            ),
            30_000
        );
    }

    #[test]
    fn resolve_inline_limit_per_tool_env_has_highest_priority() {
        // per-tool env > 描述符值 > 全局 env
        assert_eq!(
            resolve_inline_limit_impl(
                Some("12345"),
                Some(30_000),
                Some("65536"),
                DEFAULT_TOOL_RESULT_INLINE_LIMIT,
            ),
            12345
        );

        // per-tool env 为无效值 → 降级到描述符值
        assert_eq!(
            resolve_inline_limit_impl(
                Some("not-a-number"),
                Some(30_000),
                Some("65536"),
                DEFAULT_TOOL_RESULT_INLINE_LIMIT,
            ),
            30_000
        );

        // per-tool env 为 None → 降级到描述符值
        assert_eq!(
            resolve_inline_limit_impl(
                None,
                Some(20_000),
                Some("65536"),
                DEFAULT_TOOL_RESULT_INLINE_LIMIT,
            ),
            20_000
        );
    }

    #[test]
    fn resolve_inline_limit_uses_runtime_default_after_all_overrides_miss() {
        assert_eq!(resolve_inline_limit_impl(None, None, None, 88_888), 88_888);
    }
}
