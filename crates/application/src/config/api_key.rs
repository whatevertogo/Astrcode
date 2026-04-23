//! Profile API Key 解析。
//!
//! 将配置中的 API key 引用解析为实际的密钥字符串：
//! - `literal:<value>` → 直接返回
//! - `env:<NAME>` → 从环境变量读取，缺失报错
//! - 裸值 → 尝试环境变量，不存在则作为字面值回退

use astrcode_core::{AstrError, Profile, Result};

use super::env_resolver::resolve_env_value;

/// 解析 Profile 的 API key。
///
/// 支持三种格式，详见模块级文档。
pub fn resolve_api_key(profile: &Profile) -> Result<String> {
    let val = match &profile.api_key {
        None => {
            return Err(AstrError::MissingApiKey(format!(
                "profile '{}' 未配置 apiKey",
                profile.name
            )));
        },
        Some(s) => s.trim().to_string(),
    };

    if val.is_empty() {
        return Err(AstrError::MissingApiKey(format!(
            "profile '{}' 的 apiKey 不能为空",
            profile.name
        )));
    }

    let resolved = resolve_env_value(&val).map_err(|error| match error {
        AstrError::Validation(message) => {
            AstrError::Validation(format!("profile '{}' 的 apiKey {}", profile.name, message))
        },
        other => other,
    })?;

    if resolved.is_empty() {
        return Err(AstrError::MissingApiKey(format!(
            "profile '{}' 的 apiKey 解析后为空",
            profile.name
        )));
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use astrcode_core::config::{ModelConfig, OpenAiApiMode};

    use super::*;

    fn test_profile(api_key: Option<&str>) -> Profile {
        Profile {
            name: "test".to_string(),
            provider_kind: super::super::constants::PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.test.com".to_string(),
            api_key: api_key.map(|s| s.to_string()),
            models: vec![ModelConfig::new("test-model")],
            openai_capabilities: None,
            api_mode: Some(OpenAiApiMode::ChatCompletions),
        }
    }

    #[test]
    fn none_api_key_returns_error() {
        let profile = test_profile(None);
        assert!(resolve_api_key(&profile).is_err());
    }

    #[test]
    fn empty_api_key_returns_error() {
        let profile = test_profile(Some(""));
        assert!(resolve_api_key(&profile).is_err());
    }

    #[test]
    fn literal_api_key_resolved() {
        let profile = test_profile(Some("literal:sk-123"));
        assert_eq!(
            resolve_api_key(&profile).expect("literal key should resolve"),
            "sk-123"
        );
    }
}
