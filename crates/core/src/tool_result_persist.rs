//! 工具结果持久化共享契约。

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_persisted_output_detects_tag() {
        assert!(is_persisted_output(
            "<persisted-output>\nsome content\n</persisted-output>"
        ));
        assert!(!is_persisted_output("normal tool output"));
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
