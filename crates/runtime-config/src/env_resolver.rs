//! 运行时配置字段的环境变量解析。
//!
//! 本模块集中处理配置值中环境变量引用的解析逻辑，确保 `runtime-config` crate
//! 内部对哪些字符串可以逃逸配置文件、哪些字符串需要进程环境变量保持一致的判断标准。
//!
//! # 解析流程
//!
//! 1. [`parse_env_value`]：将原始字符串解析为 [`ParsedEnvValue`] 枚举
//! 2. [`resolve_env_value`]：根据解析结果从环境变量或字面值获取最终值
//!
//! # 安全设计
//!
//! 环境变量值仅在内存中解析，不会被写回 `config.json`。这样密钥可以来自
//! 进程环境变量，而不会意外持久化到配置文件中。

use astrcode_core::{AstrError, Result};

use crate::constants::{ENV_REFERENCE_PREFIX, LITERAL_VALUE_PREFIX};

/// 配置值在环境变量查找前的解析形态。
///
/// 此枚举区分了三种值来源，使解析逻辑可以精确控制哪些值必须来自环境变量、
/// 哪些可以是字面值、哪些可以回退。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedEnvValue<'a> {
    /// 字面值，直接使用。
    ///
    /// 来源：`literal:` 前缀的值，或不符合环境变量命名规范的裸值。
    Literal(&'a str),
    /// 必须从指定名称的环境变量读取。
    ///
    /// 来源：`env:` 前缀的值。若环境变量不存在则报错。
    ExplicitEnv(&'a str),
    /// 兼容模式：符合环境变量命名规范的裸值。
    ///
    /// 来源：未加前缀但形如 `MY_API_KEY` 的值。尝试读取环境变量，
    /// 若不存在则作为字面值回退（兼容旧版配置行为）。
    OptionalEnv(&'a str),
}

/// 将配置值解析为字面值或环境变量引用。
///
/// 集中此逻辑以保持 `runtime-config` 内部对字符串解析的一致性：
/// 哪些字符串允许逃逸配置文件、哪些字符串需要进程环境变量。
///
/// # 解析规则
///
/// 1. `literal:` 前缀 → [`ParsedEnvValue::Literal`]
/// 2. `env:` 前缀 → [`ParsedEnvValue::ExplicitEnv`]（需验证环境变量名合法性）
/// 3. 符合环境变量命名规范 → [`ParsedEnvValue::OptionalEnv`]
/// 4. 其他 → [`ParsedEnvValue::Literal`]
pub fn parse_env_value(raw: &str) -> Result<ParsedEnvValue<'_>> {
    let trimmed = raw.trim();

    if let Some(literal) = trimmed.strip_prefix(LITERAL_VALUE_PREFIX) {
        return Ok(ParsedEnvValue::Literal(literal.trim()));
    }

    if let Some(env_name) = trimmed.strip_prefix(ENV_REFERENCE_PREFIX) {
        let env_name = env_name.trim();
        if !is_env_var_name(env_name) {
            return Err(AstrError::Validation(format!(
                "env 引用 '{}' 非法",
                env_name
            )));
        }
        return Ok(ParsedEnvValue::ExplicitEnv(env_name));
    }

    if is_env_var_name(trimmed) {
        return Ok(ParsedEnvValue::OptionalEnv(trimmed));
    }

    Ok(ParsedEnvValue::Literal(trimmed))
}

/// 将解析后的配置值解析为有效的运行时值。
///
/// 环境变量值仅在内存中物化，这样密钥可以来自进程环境变量
/// 而不会被重写回 `config.json` 中。
pub fn resolve_env_value(raw: &str) -> Result<String> {
    match parse_env_value(raw)? {
        ParsedEnvValue::Literal(value) => Ok(value.to_string()),
        ParsedEnvValue::ExplicitEnv(env_name) => std::env::var(env_name).map_err(|_| {
            AstrError::EnvVarNotFound(format!(
                "环境变量 {} 未设置。\n解决方案：\n1. 在 Windows \
                 系统属性中设置用户环境变量（需重启应用）\n2. 或在配置文件中使用 \
                 literal:YOUR_API_KEY 直接指定",
                env_name
            ))
        }),
        ParsedEnvValue::OptionalEnv(env_name) => {
            Ok(std::env::var(env_name).unwrap_or_else(|_| env_name.to_string()))
        },
    }
}

/// 构建序列化的 `env:<NAME>` 引用字符串。
///
/// 用于默认配置值中引用环境变量，如 `env:DEEPSEEK_API_KEY`。
pub fn env_reference(env_name: &str) -> String {
    format!("{ENV_REFERENCE_PREFIX}{env_name}")
}

/// 判断一个值是否看起来像环境变量名。
///
/// 环境变量名的定义：仅包含大写字母、数字和下划线，且至少包含一个下划线。
/// 例如 `MY_API_KEY` 返回 `true`，`my_key` 和 `hello` 返回 `false`。
pub fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}
