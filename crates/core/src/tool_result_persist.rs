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

/// 解析工具结果内联阈值。
///
/// 优先级（从高到低）：
/// 1. 描述符中的 `max_result_inline_size`
/// 2. 调用方注入的默认阈值（通常来自 runtime 配置）
pub fn resolve_inline_limit(descriptor_limit: Option<usize>, configured_default: usize) -> usize {
    if let Some(limit) = descriptor_limit {
        return limit;
    }

    configured_default.max(1)
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
    fn resolve_inline_limit_uses_default_when_no_override() {
        assert_eq!(
            resolve_inline_limit(None, DEFAULT_TOOL_RESULT_INLINE_LIMIT),
            DEFAULT_TOOL_RESULT_INLINE_LIMIT
        );

        assert_eq!(
            resolve_inline_limit(Some(30_000), DEFAULT_TOOL_RESULT_INLINE_LIMIT),
            30_000
        );
    }

    #[test]
    fn resolve_inline_limit_uses_runtime_default_after_all_overrides_miss() {
        assert_eq!(resolve_inline_limit(None, 88_888), 88_888);
    }
}
