//! # 环境变量常量
//!
//! 定义 Astrcode 专用的环境变量名称常量，是整个项目最低级别的环境变量来源。
//! 基础 crate 无需依赖高层配置 crate，直接通过此模块读取进程环境变量。
//! 这些常量也是环境变量名称的唯一真实来源（single source of truth）。

/// 用于覆盖正常运行时执行的 Astrcode 主目录。
pub const ASTRCODE_HOME_DIR_ENV: &str = "ASTRCODE_HOME_DIR";

/// 用于测试隔离的 Astrcode 主目录覆盖变量。
pub const ASTRCODE_TEST_HOME_ENV: &str = "ASTRCODE_TEST_HOME";

/// 添加额外的插件发现路径，使用操作系统特定路径分隔符分隔。
pub const ASTRCODE_PLUGIN_DIRS_ENV: &str = "ASTRCODE_PLUGIN_DIRS";

/// Supplies the Tauri target triple used when preparing the sidecar binary.
pub const TAURI_ENV_TARGET_TRIPLE_ENV: &str = "TAURI_ENV_TARGET_TRIPLE";

/// Default DeepSeek API key environment variable name.
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";

/// Default Anthropic API key environment variable name.
pub const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";

/// Maximum number of concurrency-safe tools that may execute in parallel within a single step.
pub const ASTRCODE_MAX_TOOL_CONCURRENCY_ENV: &str = "ASTRCODE_MAX_TOOL_CONCURRENCY";

/// 工具结果内联阈值的全局环境变量覆盖（字节）。
/// 设置后所有工具使用此值作为落盘阈值，优先级低于 per-tool 环境变量。
pub const ASTRCODE_TOOL_RESULT_INLINE_LIMIT_ENV: &str = "ASTRCODE_TOOL_RESULT_INLINE_LIMIT";

/// 工具结果内联阈值的 per-tool 环境变量前缀。
/// 拼接大写工具名使用，如 `ASTRCODE_TOOL_INLINE_LIMIT_SHELL=50000`。
/// 优先级高于全局 `ASTRCODE_TOOL_RESULT_INLINE_LIMIT` 和描述符中的 `max_result_inline_size`。
pub const ASTRCODE_TOOL_INLINE_LIMIT_PREFIX: &str = "ASTRCODE_TOOL_INLINE_LIMIT_";

/// 本地 sidecar 服务器的默认端口号。
pub const DEFAULT_LOCAL_SERVER_PORT: u16 = 62000;
