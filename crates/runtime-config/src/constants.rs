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

use crate::types::{AgentConfig, RuntimeConfig};

/// OpenAI 兼容协议 Provider 标识符。
///
/// 用于 `Profile.provider_kind` 字段，表示该 Provider 使用 OpenAI Chat Completions API 格式。
/// Deepseek 等兼容 OpenAI 接口的服务都使用此标识符。
pub const PROVIDER_KIND_OPENAI: &str = "openai-compatible";

/// Anthropic Provider 标识符。
///
/// 用于 `Profile.provider_kind` 字段，表示该 Provider 使用 Anthropic Messages API 格式。
/// 与 OpenAI 兼容协议不同，Anthropic 使用专用请求头；同时允许通过 `baseUrl`
/// 覆盖默认官方地址，接入自定义 Anthropic 兼容网关。
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

/// Anthropic Models API endpoint URL.
///
/// 用于按模型 ID 拉取权威的上下文窗口和最大输出 token 元数据。
pub const ANTHROPIC_MODELS_API_URL: &str = "https://api.anthropic.com/v1/models";

/// Anthropic API version.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

fn split_url_query(url: &str) -> (&str, Option<&str>) {
    match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    }
}

fn join_url_query(path: String, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path,
    }
}

/// 判断路径段是否像显式 API 版本号。
///
/// OpenAI 兼容网关并不都使用 `/v1`，一些第三方会暴露 `/v4`、`/v1beta` 等版本根。
/// 这里采用业界常见的宽松识别：只要段名以 `v` 开头且紧跟数字，就认为它是
/// 一个显式版本段，后续标准集合路径应挂在该版本段之下，而不是强行回退到 `/v1`。
fn looks_like_api_version_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some('v' | 'V'))
        && matches!(chars.next(), Some(ch) if ch.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// 将已包含显式版本段的 OpenAI 兼容地址标准化到目标集合路径。
///
/// 例如：
/// - `https://api.z.ai/api/coding/paas/v4` → `.../v4/chat/completions`
/// - `https://gateway.example.com/openai/v1/chat/completion` → `.../v1/chat/completions`
fn normalize_openai_versioned_base_url(trimmed: &str, collection_suffix: &str) -> Option<String> {
    let segments = trimmed.split('/').collect::<Vec<_>>();
    let version_index = segments
        .iter()
        .rposition(|segment| looks_like_api_version_segment(segment))?;

    let prefix = segments[..=version_index].join("/");
    Some(format!("{prefix}/{collection_suffix}"))
}

fn resolve_anthropic_api_collection_url(
    base_url: &str,
    collection: &'static str,
    default_url: &'static str,
) -> String {
    let (path, query) = split_url_query(base_url.trim());
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return default_url.to_string();
    }

    let this_collection_suffix = format!("/{collection}");
    if trimmed.ends_with(&this_collection_suffix) {
        return join_url_query(trimmed.to_string(), query);
    }

    let sibling_collection = if collection == "messages" {
        "models"
    } else {
        "messages"
    };
    let sibling_suffix = format!("/{sibling_collection}");
    if trimmed.ends_with(&sibling_suffix) {
        return join_url_query(
            format!(
                "{}/{}",
                trimmed.trim_end_matches(&sibling_suffix),
                collection
            ),
            query,
        );
    }

    if trimmed.ends_with("/v1") {
        return join_url_query(format!("{trimmed}/{collection}"), query);
    }

    if let Some((prefix, _tail)) = trimmed.rsplit_once("/v1/") {
        // 兼容用户直接填写完整 endpoint 或误写尾段的情况：
        // 只要已经落在 `/v1/<something>` 形态，就把尾集合标准化成目标集合，
        // 避免继续在后面重复追加 `/v1/messages` / `/v1/models`。
        return join_url_query(format!("{prefix}/v1/{collection}"), query);
    }

    join_url_query(format!("{trimmed}/v1/{collection}"), query)
}

/// 解析 Anthropic Messages API 地址。
///
/// 兼容三种写法：
/// - 空字符串：回退到官方默认地址
/// - API 根地址：如 `https://gateway.example.com/anthropic`
/// - 完整集合地址：如 `https://gateway.example.com/anthropic/v1/messages`
pub fn resolve_anthropic_messages_api_url(base_url: &str) -> String {
    resolve_anthropic_api_collection_url(base_url, "messages", ANTHROPIC_MESSAGES_API_URL)
}

/// 解析 Anthropic Models API 地址。
///
/// 与 [`resolve_anthropic_messages_api_url`] 使用同一规则，确保运行时请求消息和探测模型
/// metadata 时落在同一条自定义网关链路上。
pub fn resolve_anthropic_models_api_url(base_url: &str) -> String {
    resolve_anthropic_api_collection_url(base_url, "models", ANTHROPIC_MODELS_API_URL)
}

/// 解析 OpenAI Chat Completions API 地址。
///
/// 兼容三种写法：
/// - API 根地址：如 `https://api.deepseek.com`
/// - 版本根地址：如 `https://api.openai.com/v1`
/// - 完整集合地址：如 `https://api.openai.com/v1/chat/completions`
/// - 第三方版本根地址：如 `https://api.z.ai/api/coding/paas/v4`
///
/// 如果用户已经写到 `/vN/<something>`，这里会把尾段标准化成
/// `/vN/chat/completions`，避免再次拼接出重复后缀，也避免把第三方显式版本
/// 根错误改写成 `/v1/...`。
pub fn resolve_openai_chat_completions_api_url(base_url: &str) -> String {
    let (path, query) = split_url_query(base_url.trim());
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized = if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/chat") {
        format!("{trimmed}/completions")
    } else if let Some(versioned_url) =
        normalize_openai_versioned_base_url(trimmed, "chat/completions")
    {
        versioned_url
    } else {
        format!("{trimmed}/v1/chat/completions")
    };

    join_url_query(normalized, query)
}

/// OpenAI-compatible 模型的保守默认上下文窗口。
///
/// 用于默认生成的 OpenAI-compatible profile，避免首次创建配置文件时出现空 limits。
pub const DEFAULT_OPENAI_CONTEXT_LIMIT: usize = 128_000;

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

// ============================================================================
// LLM 客户端配置
// ============================================================================

/// 默认 LLM 连接超时（秒）。
///
/// 建立 TCP 连接的最大等待时间，超时后返回错误。
/// 网络不稳定时可适当调大此值。
pub const DEFAULT_LLM_CONNECT_TIMEOUT_SECS: u64 = 10;

/// 默认 LLM 读取超时（秒）。
///
/// 等待响应流的最大时间，需要足够长以支持慢速流式响应，
/// 但也要能检测到卡死的连接。
pub const DEFAULT_LLM_READ_TIMEOUT_SECS: u64 = 90;

/// 默认 LLM 请求最大重试次数。
///
/// 针对瞬态故障（408、429、5xx）的自动重试上限。
pub const DEFAULT_LLM_MAX_RETRIES: u32 = 2;

/// 默认 LLM 重试基础延迟（毫秒）。
///
/// 首次重试前的等待时间，后续重试采用指数退避。
pub const DEFAULT_LLM_RETRY_BASE_DELAY_MS: u64 = 250;

// ============================================================================
// Agent 循环配置
// ============================================================================

/// 默认响应式压缩最大重试次数。
///
/// 当 LLM 返回 413 prompt-too-long 时触发的压缩重试上限。
/// 超过此次数仍失败则向用户报告错误。
pub const DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS: u8 = 3;

/// 默认输出续调最大尝试次数。
///
/// 当模型输出被 max_tokens 截断时，自动续调的最大次数。
pub const DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS: u8 = 3;

/// 默认摘要保留 token 数。
///
/// 压缩时为摘要生成预留的 token 预算，避免摘要过长挤占上下文。
pub const DEFAULT_SUMMARY_RESERVE_TOKENS: usize = 20_000;

/// 默认最大跟踪文件数。
///
/// FileAccessTracker 跟踪的最近访问文件数上限，用于压缩后恢复上下文。
pub const DEFAULT_MAX_TRACKED_FILES: usize = 10;

/// 默认压缩恢复最大文件数。
///
/// Post-compact 文件恢复时最多读取的文件数。
pub const DEFAULT_MAX_RECOVERED_FILES: usize = 5;

/// 默认恢复 token 预算。
///
/// Post-compact 文件恢复的总 token 预算，避免恢复过多内容。
pub const DEFAULT_RECOVERY_TOKEN_BUDGET: usize = 50_000;

// ============================================================================
// 工具限制配置
// ============================================================================

/// 默认工具结果内联阈值（字节）。
///
/// 工具输出超过此大小时存盘，仅在消息中保留预览。
pub const DEFAULT_TOOL_RESULT_INLINE_LIMIT: usize = 32 * 1024;

/// 默认工具结果预览限制（字节）。
///
/// 存盘时返回的预览内容大小，供 LLM 快速了解输出性质。
pub const DEFAULT_TOOL_RESULT_PREVIEW_LIMIT: usize = 2 * 1024;

/// 默认最大图片文件大小（字节）。
///
/// readFile 读取图片时的最大允许大小，超过则拒绝读取。
pub const DEFAULT_MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024;

/// 默认 grep 最大显示行数。
///
/// grep 工具单次返回的最大行数，避免超大输出。
pub const DEFAULT_MAX_GREP_LINES: usize = 500;

// ============================================================================
// 会话配置
// ============================================================================

/// 默认会话广播容量。
///
/// broadcast channel 容量，慢速 SSE 客户端若未在此数量内消费，
/// 旧事件会被丢弃。2048 足够覆盖一次完整 turn。
pub const DEFAULT_SESSION_BROADCAST_CAPACITY: usize = 2048;

/// 默认会话最近记录限制。
///
/// 内存中保留的最近事件记录数，超过时从头部淘汰。
/// 4096 约覆盖 40-50 次典型 turn。
pub const DEFAULT_SESSION_RECENT_RECORD_LIMIT: usize = 4096;

/// 默认最大并发分支深度。
///
/// 当向正在运行的会话提交新 Prompt 时，自动创建分支的最大深度。
/// 超过此深度拒绝提交，防止分支树膨胀。
pub const DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH: usize = 3;

// ============================================================================
// 服务器认证配置
// ============================================================================

/// 默认 API 会话有效期（小时）。
///
/// 通过 /api/auth/exchange 获得的 token 有效期。
pub const DEFAULT_API_SESSION_TTL_HOURS: i64 = 8;

// ============================================================================
// 多 Agent 控制配置
// ============================================================================

/// 默认 Agent 嵌套深度上限。
///
/// 限制子 Agent 可被嵌套的层数，防止无限递归。
/// 例如 maxDepth=3 表示最多允许 root→child→grandchild 三层。
pub const DEFAULT_MAX_AGENT_DEPTH: usize = 3;

/// 默认受控子会话最大深度。
///
/// 默认值刻意比旧多 Agent 原型更保守，防止子 agent 再起子 agent
/// 过早把事件流和 UI 复杂度推高。
pub const DEFAULT_MAX_SUBRUN_DEPTH: usize = 1;

/// 默认并发子 Agent 数上限。
///
/// 同时处于活跃状态的子 Agent 最大数量。
pub const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 5;

/// 默认终态 Agent 保留数。
///
/// 已完成/已失败/已取消的 Agent 条目在内存中的保留上限。
pub const DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT: usize = 256;

/// 是否默认启用独立子会话实验特性。
pub const DEFAULT_EXPERIMENTAL_INDEPENDENT_SESSION: bool = false;

// ============================================================================
// 压缩恢复与熔断配置
// ============================================================================

/// 默认熔断阈值：连续压缩失败次数达到此值后暂停自动压缩。
///
/// 网络不稳定环境下可适当调大此值，避免过早熔断导致上下文无法压缩。
pub const DEFAULT_MAX_CONSECUTIVE_FAILURES: usize = 3;

/// 默认压缩恢复文件内容的截断字节数。
///
/// Post-compact 文件恢复时注入的文件内容超过此大小会被截断，
/// 防止单个文件占用过多上下文窗口。
pub const DEFAULT_RECOVERY_TRUNCATE_BYTES: usize = 30_000;

// ============================================================================
// 预留给未来多智能体消息传递功能
// ============================================================================

// TODO(multi-agent): 以下常量随 P2 的 Agent 间消息传递功能一起实现：
// - DEFAULT_AGENT_MESSAGE_QUEUE_CAPACITY: agent 间消息队列容量
// - DEFAULT_AGENT_COORDINATION_TIMEOUT_SECS: agent 协调超时

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

// ============================================================================
// 多 Agent 控制配置解析
// ============================================================================

/// 解析 Agent 嵌套深度上限。
///
/// 当 `runtime.agent.maxDepth` 未设置时回退到 [`DEFAULT_MAX_AGENT_DEPTH`]。
pub fn resolve_agent_max_depth(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.max_depth)
        .unwrap_or(DEFAULT_MAX_AGENT_DEPTH)
        .max(1)
}

/// 解析受控子会话最大深度。
///
/// 新逻辑优先读取 `maxSubrunDepth`，并在旧配置里回退到 `maxDepth`，
/// 这样可以平滑兼容已经存在的用户配置。
pub fn resolve_agent_max_subrun_depth(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.max_subrun_depth.or(a.max_depth))
        .unwrap_or(DEFAULT_MAX_SUBRUN_DEPTH)
        .max(1)
}

/// 解析并发子 Agent 数上限。
///
/// 当 `runtime.agent.maxConcurrent` 未设置时回退到 [`DEFAULT_MAX_CONCURRENT_AGENTS`]。
pub fn resolve_agent_max_concurrent(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.max_concurrent)
        .unwrap_or(DEFAULT_MAX_CONCURRENT_AGENTS)
        .max(1)
}

/// 解析终态 Agent 保留数。
///
/// 当 `runtime.agent.finalizedRetainLimit` 未设置时回退到
/// [`DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT`]。
pub fn resolve_agent_finalized_retain_limit(agent: Option<&AgentConfig>) -> usize {
    agent
        .and_then(|a| a.finalized_retain_limit)
        .unwrap_or(DEFAULT_FINALIZED_AGENT_RETAIN_LIMIT)
}

/// 解析是否启用独立子会话实验特性。
pub fn resolve_agent_experimental_independent_session(agent: Option<&AgentConfig>) -> bool {
    agent
        .and_then(|a| a.experimental_independent_session)
        .unwrap_or(DEFAULT_EXPERIMENTAL_INDEPENDENT_SESSION)
}

// ============================================================================
// 压缩恢复与熔断配置解析
// ============================================================================

/// 解析熔断阈值（连续压缩失败次数）。
///
/// 当 `runtime.maxConsecutiveFailures` 未设置时回退到 [`DEFAULT_MAX_CONSECUTIVE_FAILURES`]。
pub fn resolve_max_consecutive_failures(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_consecutive_failures
        .unwrap_or(DEFAULT_MAX_CONSECUTIVE_FAILURES)
        .max(1)
}

/// 解析压缩恢复文件内容的截断字节数。
///
/// 当 `runtime.recoveryTruncateBytes` 未设置时回退到 [`DEFAULT_RECOVERY_TRUNCATE_BYTES`]。
pub fn resolve_recovery_truncate_bytes(runtime: &RuntimeConfig) -> usize {
    runtime
        .recovery_truncate_bytes
        .unwrap_or(DEFAULT_RECOVERY_TRUNCATE_BYTES)
        .max(1024) // 至少 1KB
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

// ============================================================================
// LLM 客户端配置解析
// ============================================================================

/// 解析 LLM 连接超时（秒）。
pub fn resolve_llm_connect_timeout_secs(runtime: &RuntimeConfig) -> u64 {
    runtime
        .llm_connect_timeout_secs
        .unwrap_or(DEFAULT_LLM_CONNECT_TIMEOUT_SECS)
        .max(1)
}

/// 解析 LLM 读取超时（秒）。
pub fn resolve_llm_read_timeout_secs(runtime: &RuntimeConfig) -> u64 {
    runtime
        .llm_read_timeout_secs
        .unwrap_or(DEFAULT_LLM_READ_TIMEOUT_SECS)
        .max(1)
}

/// 解析 LLM 最大重试次数。
pub fn resolve_llm_max_retries(runtime: &RuntimeConfig) -> u32 {
    runtime.llm_max_retries.unwrap_or(DEFAULT_LLM_MAX_RETRIES)
}

// ============================================================================
// Agent 循环配置解析
// ============================================================================

/// 解析响应式压缩最大重试次数。
pub fn resolve_max_reactive_compact_attempts(runtime: &RuntimeConfig) -> u8 {
    runtime
        .max_reactive_compact_attempts
        .unwrap_or(DEFAULT_MAX_REACTIVE_COMPACT_ATTEMPTS)
        .max(1)
}

/// 解析输出续调最大尝试次数。
pub fn resolve_max_output_continuation_attempts(runtime: &RuntimeConfig) -> u8 {
    runtime
        .max_output_continuation_attempts
        .unwrap_or(DEFAULT_MAX_OUTPUT_CONTINUATION_ATTEMPTS)
        .max(1)
}

/// 解析摘要保留 token 数。
pub fn resolve_summary_reserve_tokens(runtime: &RuntimeConfig) -> usize {
    runtime
        .summary_reserve_tokens
        .unwrap_or(DEFAULT_SUMMARY_RESERVE_TOKENS)
}

/// 解析最大跟踪文件数。
pub fn resolve_max_tracked_files(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_tracked_files
        .unwrap_or(DEFAULT_MAX_TRACKED_FILES)
        .max(1)
}

/// 解析压缩恢复最大文件数。
pub fn resolve_max_recovered_files(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_recovered_files
        .unwrap_or(DEFAULT_MAX_RECOVERED_FILES)
}

/// 解析恢复 token 预算。
pub fn resolve_recovery_token_budget(runtime: &RuntimeConfig) -> usize {
    runtime
        .recovery_token_budget
        .unwrap_or(DEFAULT_RECOVERY_TOKEN_BUDGET)
}

// ============================================================================
// 工具限制配置解析
// ============================================================================

/// 解析工具结果内联阈值（字节）。
pub fn resolve_tool_result_inline_limit(runtime: &RuntimeConfig) -> usize {
    runtime
        .tool_result_inline_limit
        .unwrap_or(DEFAULT_TOOL_RESULT_INLINE_LIMIT)
        .max(1024) // 至少 1KB
}

/// 解析工具结果预览限制（字节）。
pub fn resolve_tool_result_preview_limit(runtime: &RuntimeConfig) -> usize {
    runtime
        .tool_result_preview_limit
        .unwrap_or(DEFAULT_TOOL_RESULT_PREVIEW_LIMIT)
        .max(256) // 至少 256 字节
}

/// 解析最大图片文件大小（字节）。
pub fn resolve_max_image_size(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_image_size
        .unwrap_or(DEFAULT_MAX_IMAGE_SIZE)
        .max(1024 * 1024) // 至少 1MB
}

/// 解析 grep 最大显示行数。
pub fn resolve_max_grep_lines(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_grep_lines
        .unwrap_or(DEFAULT_MAX_GREP_LINES)
        .max(10)
}

// ============================================================================
// 会话配置解析
// ============================================================================

/// 解析会话广播容量。
pub fn resolve_session_broadcast_capacity(runtime: &RuntimeConfig) -> usize {
    runtime
        .session_broadcast_capacity
        .unwrap_or(DEFAULT_SESSION_BROADCAST_CAPACITY)
        .max(64)
}

/// 解析会话最近记录限制。
pub fn resolve_session_recent_record_limit(runtime: &RuntimeConfig) -> usize {
    runtime
        .session_recent_record_limit
        .unwrap_or(DEFAULT_SESSION_RECENT_RECORD_LIMIT)
        .max(128)
}

/// 解析最大并发分支深度。
pub fn resolve_max_concurrent_branch_depth(runtime: &RuntimeConfig) -> usize {
    runtime
        .max_concurrent_branch_depth
        .unwrap_or(DEFAULT_MAX_CONCURRENT_BRANCH_DEPTH)
        .max(1)
}

// ============================================================================
// 服务器认证配置解析
// ============================================================================

/// 解析 API 会话有效期（小时）。
pub fn resolve_api_session_ttl_hours(runtime: &RuntimeConfig) -> i64 {
    runtime
        .api_session_ttl_hours
        .unwrap_or(DEFAULT_API_SESSION_TTL_HOURS)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::{
        ANTHROPIC_MESSAGES_API_URL, ANTHROPIC_MODELS_API_URL, resolve_anthropic_messages_api_url,
        resolve_anthropic_models_api_url, resolve_openai_chat_completions_api_url,
    };

    #[test]
    fn anthropic_url_helpers_fall_back_to_official_defaults() {
        assert_eq!(
            resolve_anthropic_messages_api_url(""),
            ANTHROPIC_MESSAGES_API_URL
        );
        assert_eq!(
            resolve_anthropic_models_api_url(""),
            ANTHROPIC_MODELS_API_URL
        );
    }

    #[test]
    fn anthropic_url_helpers_expand_root_style_base_url() {
        let base_url = "https://gateway.example.com/anthropic";

        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models"
        );
    }

    #[test]
    fn anthropic_url_helpers_accept_full_collection_urls() {
        let messages_url = "https://gateway.example.com/anthropic/v1/messages";
        let models_url = "https://gateway.example.com/anthropic/v1/models";

        assert_eq!(
            resolve_anthropic_messages_api_url(messages_url),
            messages_url
        );
        assert_eq!(resolve_anthropic_models_api_url(messages_url), models_url);
        assert_eq!(resolve_anthropic_models_api_url(models_url), models_url);
        assert_eq!(resolve_anthropic_messages_api_url(models_url), messages_url);
    }

    #[test]
    fn anthropic_url_helpers_trim_whitespace_and_trailing_slashes() {
        let base_url = "  https://gateway.example.com/anthropic/v1/  ";

        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models"
        );
    }

    #[test]
    fn anthropic_url_helpers_expand_v1_base_without_collection() {
        let base_url = "https://gateway.example.com/anthropic/v1";

        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models"
        );
    }

    #[test]
    fn anthropic_url_helpers_replace_nonstandard_v1_tail_instead_of_appending_again() {
        let base_url = "https://gateway.example.com/anthropic/v1/messeges?foo=bar";

        assert_eq!(
            resolve_anthropic_messages_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/messages?foo=bar"
        );
        assert_eq!(
            resolve_anthropic_models_api_url(base_url),
            "https://gateway.example.com/anthropic/v1/models?foo=bar"
        );
    }

    #[test]
    fn openai_url_helper_expands_root_style_base_url() {
        assert_eq!(
            resolve_openai_chat_completions_api_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_openai_chat_completions_api_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_helper_preserves_non_v1_version_root_for_third_party_gateways() {
        assert_eq!(
            resolve_openai_chat_completions_api_url("https://api.z.ai/api/coding/paas/v4"),
            "https://api.z.ai/api/coding/paas/v4/chat/completions"
        );
    }

    #[test]
    fn openai_url_helper_normalizes_non_v1_versioned_endpoint_tails() {
        assert_eq!(
            resolve_openai_chat_completions_api_url(
                "https://api.z.ai/api/coding/paas/v4/chat/completion?foo=bar"
            ),
            "https://api.z.ai/api/coding/paas/v4/chat/completions?foo=bar"
        );
    }

    #[test]
    fn openai_url_helper_preserves_full_endpoint_and_query() {
        assert_eq!(
            resolve_openai_chat_completions_api_url(
                "https://gateway.example.com/openai/v1/chat/completions?foo=bar"
            ),
            "https://gateway.example.com/openai/v1/chat/completions?foo=bar"
        );
    }

    #[test]
    fn openai_url_helper_replaces_nonstandard_v1_tail_instead_of_duplicating_suffix() {
        assert_eq!(
            resolve_openai_chat_completions_api_url(
                "https://gateway.example.com/openai/v1/chat/completion"
            ),
            "https://gateway.example.com/openai/v1/chat/completions"
        );
    }
}
