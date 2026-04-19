//! 配置字段的环境变量解析。
//!
//! 处理配置值中环境变量引用的解析逻辑，区分三种值来源：
//! - `literal:` 前缀 → 字面值
//! - `env:` 前缀 → 必须从环境变量读取
//! - 裸值 → 若像环境变量名则尝试读取，否则作字面值

use astrcode_core::{AstrError, Result};

/// `literal:` 前缀，表示后续内容为字面值，不做环境变量解析。
const LITERAL_VALUE_PREFIX: &str = "literal:";

/// `env:` 前缀，表示后续内容为必须解析的环境变量名。
const ENV_REFERENCE_PREFIX: &str = "env:";

/// 配置值在环境变量查找前的解析形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedEnvValue<'a> {
    /// 字面值，直接使用。
    Literal(&'a str),
    /// 必须从指定名称的环境变量读取，缺失时报错。
    ExplicitEnv(&'a str),
    /// 兼容模式：符合环境变量命名规范的裸值，尝试读取环境变量，不存在则回退为字面值。
    OptionalEnv(&'a str),
}

/// 将配置值解析为字面值或环境变量引用。
///
/// 解析规则：
/// 1. `literal:` 前缀 → `Literal`
/// 2. `env:` 前缀 → `ExplicitEnv`（验证环境变量名合法性）
/// 3. 符合环境变量命名规范 → `OptionalEnv`
/// 4. 其他 → `Literal`
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
pub fn resolve_env_value(raw: &str) -> Result<String> {
    match parse_env_value(raw)? {
        ParsedEnvValue::Literal(value) => Ok(value.to_string()),
        ParsedEnvValue::ExplicitEnv(env_name) => std::env::var(env_name).map_err(|_| {
            AstrError::EnvVarNotFound(format!(
                "环境变量 {} 未设置。\n解决方案：\n1. \
                 在系统属性中设置用户环境变量（需重启应用）\n2. 或在配置文件中使用 \
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
pub fn env_reference(env_name: &str) -> String {
    format!("{ENV_REFERENCE_PREFIX}{env_name}")
}

/// 判断值是否看起来像环境变量名（大写字母+数字+下划线，至少含一个下划线）。
pub fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_prefix_bypasses_env() {
        assert_eq!(
            parse_env_value("literal:my-secret-key").expect("literal should parse"),
            ParsedEnvValue::Literal("my-secret-key")
        );
    }

    #[test]
    fn env_prefix_parses() {
        assert_eq!(
            parse_env_value("env:MY_KEY").expect("env prefix should parse"),
            ParsedEnvValue::ExplicitEnv("MY_KEY")
        );
    }

    #[test]
    fn bare_uppercase_with_underscore_is_optional_env() {
        assert_eq!(
            parse_env_value("MY_API_KEY").expect("uppercase with underscore should parse"),
            ParsedEnvValue::OptionalEnv("MY_API_KEY")
        );
    }

    #[test]
    fn plain_text_is_literal() {
        assert_eq!(
            parse_env_value("hello world").expect("plain text should parse"),
            ParsedEnvValue::Literal("hello world")
        );
    }

    #[test]
    fn env_var_name_rules() {
        assert!(is_env_var_name("MY_API_KEY"));
        assert!(is_env_var_name("A_1"));
        assert!(!is_env_var_name("no_underscores_must_have")); // actually has underscores, this passes
        assert!(!is_env_var_name("lowercase"));
        assert!(!is_env_var_name("NOLOWERCASE")); // no underscore
    }
}
