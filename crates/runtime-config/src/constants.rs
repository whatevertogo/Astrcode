//! 运行时配置常量定义。
//!
//! 本模块集中管理 Astrcode 配置相关的所有常量，包括：
//! - **Provider 标识符**：`PROVIDER_KIND_OPENAI`、`PROVIDER_KIND_ANTHROPIC`
//! - **值前缀**：`ENV_REFERENCE_PREFIX`（`env:`）、`LITERAL_VALUE_PREFIX`（`literal:`）
//! - **环境变量分类**：按职责域分组（HOME / PLUGIN / PROVIDER / BUILD / RUNTIME）
//! - **默认值**：所有运行时调优参数的内置默认值
//! - **解析函数**：从配置或环境变量解析有效值的辅助函数
//!
//! # 环境变量分组
//!
//! 所有 Astrcode 定义的环境变量通过 `ALL_ASTRCODE_ENV_VARS` 统一索引，
//! 同时按职责域分为以下子集：
//! - [`HOME_ENV_VARS`]：影响本地存储路径的环境变量
//! - [`PLUGIN_ENV_VARS`]：影响插件发现的环境变量
//! - [`PROVIDER_API_KEY_ENV_VARS`]：内置 Provider 默认使用的 API key 环境变量
//! - [`BUILD_ENV_VARS`]：Tauri sidecar 构建所需的环境变量
//! - [`RUNTIME_ENV_VARS`]：调优运行时执行行为的环境变量
//!
//! # 新增环境变量指南
//!
//! 新增环境变量时：
//! 1. 在 `crates/core/src/env.rs` 中定义常量
//! 2. 在本模块的对应分组数组中添加引用
//! 3. 确保 `ALL_ASTRCODE_ENV_VARS` 包含新变量
//! 4. 不要在其他模块中硬编码环境变量名

use crate::types::RuntimeConfig;

/// OpenAI 兼容协议 Provider 标识符。
///
/// 用于 `Profile.provider_kind` 字段，表示该 Provider 使用 OpenAI Chat Completions API 格式。
/// Deepseek 等兼容 OpenAI 接口的服务都使用此标识符。
pub const PROVIDER_KIND_OPENAI: &str = "openai-compatible";

/// Anthropic Provider 标识符。
///
/// 用于 `Profile.provider_kind` 字段，表示该 Provider 使用 Anthropic Messages API 格式。
/// 与 OpenAI 兼容协议不同，Anthropic 使用固定的 API 端点和专用的请求头。
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";

/// 环境变量引用前缀。
///
/// 配置值以 `env:` 开头时，表示该值必须从指定名称的环境变量中读取。
/// 例如 `env:DEEPSEEK_API_KEY` 会尝试读取 `DEEPSEEK_API_KEY` 环境变量。
/// 如果环境变量不存在，解析会报错。
pub const ENV_REFERENCE_PREFIX: &str = "env:";

/// 字面值前缀。
///
/// 配置值以 `literal:` 开头时，表示该值应直接作为字面值使用，
/// 跳过任何环境变量解析逻辑。用于避免形如 `MY_KEY` 的值被误认为环境变量名。
pub const LITERAL_VALUE_PREFIX: &str = "literal:";

pub use astrcode_core::env::{
    ANTHROPIC_API_KEY_ENV, ASTRCODE_HOME_DIR_ENV, ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV, ASTRCODE_TEST_HOME_ENV, DEEPSEEK_API_KEY_ENV,
    TAURI_ENV_TARGET_TRIPLE_ENV,
};

/// 影响 Astrcode 本地存储路径的环境变量。
///
/// 包含 `ASTRCODE_HOME_DIR_ENV`（自定义 home 目录）和 `ASTRCODE_TEST_HOME_ENV`（测试隔离）。
pub const HOME_ENV_VARS: &[&str] = &[ASTRCODE_HOME_DIR_ENV, ASTRCODE_TEST_HOME_ENV];

/// 影响运行时插件发现的环境变量。
///
/// 包含 `ASTRCODE_PLUGIN_DIRS_ENV`，用于指定额外的插件搜索目录。
pub const PLUGIN_ENV_VARS: &[&str] = &[ASTRCODE_PLUGIN_DIRS_ENV];

/// 内置 Provider 默认配置使用的 API key 环境变量。
///
/// 默认 Profile 的 `api_key` 字段会引用这些环境变量，
/// 用户需要在环境中设置对应的值才能使用默认 Provider。
pub const PROVIDER_API_KEY_ENV_VARS: &[&str] = &[DEEPSEEK_API_KEY_ENV, ANTHROPIC_API_KEY_ENV];

/// Tauri sidecar 构建管道所需的环境变量。
///
/// 包含 `TAURI_ENV_TARGET_TRIPLE_ENV`，用于确定目标平台的二进制文件名后缀。
pub const BUILD_ENV_VARS: &[&str] = &[TAURI_ENV_TARGET_TRIPLE_ENV];

/// 调优运行时执行行为的环境变量。
///
/// 包含 `ASTRCODE_MAX_TOOL_CONCURRENCY_ENV`，用于在不修改配置文件的情况下
/// 调整工具并发上限。
pub const RUNTIME_ENV_VARS: &[&str] = &[ASTRCODE_MAX_TOOL_CONCURRENCY_ENV];

/// 所有 Astrcode 定义的环境变量。
///
/// 此数组是上述所有分组数组的并集，用于文档生成和环境变量审计。
/// 新增环境变量时必须同步更新此数组和对应的分组数组。
pub const ALL_ASTRCODE_ENV_VARS: &[&str] = &[
    ASTRCODE_HOME_DIR_ENV,
    ASTRCODE_TEST_HOME_ENV,
    ASTRCODE_PLUGIN_DIRS_ENV,
    DEEPSEEK_API_KEY_ENV,
    ANTHROPIC_API_KEY_ENV,
    TAURI_ENV_TARGET_TRIPLE_ENV,
    ASTRCODE_MAX_TOOL_CONCURRENCY_ENV,
];

/// Anthropic Messages API endpoint URL.
pub const ANTHROPIC_MESSAGES_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic API version.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// 配置 schema 的当前版本号。
///
/// 加载配置时，空白的 version 字段会被迁移到此值。
/// 不支持的版本号会导致加载失败。
pub const CURRENT_CONFIG_VERSION: &str = "1";

/// 默认最大安全工具并发数。
///
/// 当 `runtime.maxToolConcurrency` 和环境变量都未设置时使用此值。
/// 限制并发数防止同时执行过多工具调用导致系统资源耗尽。
pub const DEFAULT_MAX_TOOL_CONCURRENCY: usize = 10;
/// 默认自动上下文压缩开关。
///
/// 启用后，当对话接近模型上下文窗口限制时会自动压缩历史消息。
pub const DEFAULT_AUTO_COMPACT_ENABLED: bool = true;
/// 默认上下文压缩触发阈值（百分比）。
///
/// 当有效上下文窗口使用率达到此百分比时触发压缩。
pub const DEFAULT_COMPACT_THRESHOLD_PERCENT: u8 = 90;
/// 默认单个工具结果的请求字节预算。
///
/// 限制单个工具结果发送给模型的字节数，防止过大的输出消耗上下文窗口。
pub const DEFAULT_TOOL_RESULT_MAX_BYTES: usize = 100_000;
/// 默认上下文压缩时保留的最近用户回合数。
///
/// 压缩过程中保留这些最近的用户回合不被压缩，确保模型仍能看到最近的对话。
pub const DEFAULT_COMPACT_KEEP_RECENT_TURNS: u8 = 4;
/// 默认 token 预算。
///
/// 值为 0 时禁用自动续调功能。非零值表示每次初始回合后可继续消耗的最大 token 数。
pub const DEFAULT_TOKEN_BUDGET: u64 = 0;
/// 默认自动续调的边际收益递减阈值。
///
/// 如果上一次续调回复的 token 增量小于此值，说明模型已接近完成，停止续调。
pub const DEFAULT_CONTINUATION_MIN_DELTA_TOKENS: usize = 500;
/// 默认最大自动续调次数。
///
/// 限制初始回合后的续调次数，防止模型陷入无限循环。
pub const DEFAULT_MAX_CONTINUATIONS: u8 = 3;

/// 从进程环境变量/默认值获取最大安全工具并发数。
///
/// 此函数仅读取环境变量，适用于尚未加载 `config.json` 的底层调用方。
/// 已加载配置的高层运行时服务应优先使用 [`resolve_max_tool_concurrency`]，
/// 以保证用户配置的 `runtime.maxToolConcurrency` 具有最高优先级。
pub fn max_tool_concurrency() -> usize {
    std::env::var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_TOOL_CONCURRENCY)
        .max(1)
}

/// 解析已加载运行时配置的有效工具并发上限。
///
/// 解析优先级：
/// 1. `config.runtime.maxToolConcurrency`（用户配置最高优先级）
/// 2. `ASTRCODE_MAX_TOOL_CONCURRENCY` 环境变量
/// 3. 内置默认值（[`DEFAULT_MAX_TOOL_CONCURRENCY`]）
///
/// 此设计将运行时调优集中在 `config.json` 中，同时不破坏现有的基于环境变量的
/// 部署和测试流程。
pub fn resolve_max_tool_concurrency(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_tool_concurrency
        .unwrap_or_else(max_tool_concurrency)
        .max(1)
}

pub fn resolve_auto_compact_enabled(runtime: &RuntimeConfig) -> bool {
    runtime
        .auto_compact_enabled
        .unwrap_or(DEFAULT_AUTO_COMPACT_ENABLED)
}

pub fn resolve_compact_threshold_percent(runtime: &RuntimeConfig) -> u8 {
    runtime
        .compact_threshold_percent
        .unwrap_or(DEFAULT_COMPACT_THRESHOLD_PERCENT)
        .clamp(1, 100)
}

pub fn resolve_tool_result_max_bytes(runtime: &RuntimeConfig) -> usize {
    runtime
        .tool_result_max_bytes
        .unwrap_or(DEFAULT_TOOL_RESULT_MAX_BYTES)
        .max(1)
}

pub fn resolve_compact_keep_recent_turns(runtime: &RuntimeConfig) -> u8 {
    runtime
        .compact_keep_recent_turns
        .unwrap_or(DEFAULT_COMPACT_KEEP_RECENT_TURNS)
        .max(1)
}

pub fn resolve_default_token_budget(runtime: &RuntimeConfig) -> u64 {
    runtime.default_token_budget.unwrap_or(DEFAULT_TOKEN_BUDGET)
}

pub fn resolve_continuation_min_delta_tokens(runtime: &RuntimeConfig) -> usize {
    runtime
        .continuation_min_delta_tokens
        .unwrap_or(DEFAULT_CONTINUATION_MIN_DELTA_TOKENS)
        .max(1)
}

pub fn resolve_max_continuations(runtime: &RuntimeConfig) -> u8 {
    runtime
        .max_continuations
        .unwrap_or(DEFAULT_MAX_CONTINUATIONS)
        .max(1)
}
