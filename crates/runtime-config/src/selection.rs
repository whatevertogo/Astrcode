//! 配置选择辅助函数。
//!
//! 本模块封装 Profile 和 Model 的选择与回退逻辑，确保所有调用方都使用同一套解析规则，
//! 而不是各自根据字符串列表重新实现一遍。

use astrcode_core::{AstrError, Result};

use crate::{Config, ModelConfig, Profile};

/// 应用 Profile/Model 回退后的最终选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSelection {
    pub active_profile: String,
    pub active_model: String,
    pub warning: Option<String>,
}

/// 运行时当前将使用的有效模型信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentModelSelection {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

/// 运行时当前将使用的完整模型配置。
///
/// 这个结构给 provider 工厂和其他需要 limits 的调用方使用，避免它们再根据 model ID
/// 反查一次配置，从而把“活跃模型解析”逻辑固定在一处。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelConfig {
    pub profile_name: String,
    pub provider_kind: String,
    pub profile_base_url: String,
    pub profile_api_key: Option<String>,
    pub model: ModelConfig,
}

/// 扁平化的模型选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

pub fn resolve_active_selection(
    active_profile: &str,
    active_model: &str,
    profiles: &[Profile],
) -> Result<ActiveSelection> {
    let fallback_profile = profiles
        .first()
        .ok_or_else(|| AstrError::Internal("no profiles configured".to_string()))?;

    let selected_profile = profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .unwrap_or(fallback_profile);

    let fallback_model = selected_profile.models.first().ok_or_else(|| {
        AstrError::Internal(format!("profile '{}' has no models", selected_profile.name))
    })?;

    if selected_profile.name != active_profile {
        return Ok(ActiveSelection {
            active_profile: selected_profile.name.clone(),
            active_model: fallback_model.id.clone(),
            warning: Some(format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            )),
        });
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|model| model.id == active_model)
    {
        return Ok(ActiveSelection {
            active_profile: selected_profile.name.clone(),
            active_model: model.id.clone(),
            warning: None,
        });
    }

    Ok(ActiveSelection {
        active_profile: selected_profile.name.clone(),
        active_model: fallback_model.id.clone(),
        warning: Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, fallback_model.id
        )),
    })
}

pub fn resolve_current_model(config: &Config) -> Result<CurrentModelSelection> {
    let resolved = resolve_selected_model_config(config)?;
    Ok(CurrentModelSelection {
        profile_name: resolved.profile_name,
        model: resolved.model.id,
        provider_kind: resolved.provider_kind,
    })
}

pub fn resolve_selected_model_config(config: &Config) -> Result<ResolvedModelConfig> {
    let profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == config.active_profile)
        .or_else(|| config.profiles.first())
        .ok_or_else(|| AstrError::Internal("no profiles configured".to_string()))?;

    let model = resolve_model_for_profile(profile, &config.active_model)?
        .cloned()
        .ok_or_else(|| AstrError::Internal(format!("profile '{}' has no models", profile.name)))?;

    Ok(ResolvedModelConfig {
        profile_name: profile.name.clone(),
        provider_kind: profile.provider_kind.clone(),
        profile_base_url: profile.base_url.clone(),
        profile_api_key: profile.api_key.clone(),
        model,
    })
}

pub fn resolve_model_for_profile<'a>(
    profile: &'a Profile,
    active_model: &str,
) -> Result<Option<&'a ModelConfig>> {
    if profile.models.is_empty() {
        return Err(AstrError::Internal(format!(
            "profile '{}' has no models",
            profile.name
        )));
    }

    Ok(profile
        .models
        .iter()
        .find(|model| model.id == active_model)
        .or_else(|| profile.models.first()))
}

pub fn list_model_options(config: &Config) -> Vec<ModelOption> {
    config
        .profiles
        .iter()
        .flat_map(|profile| {
            profile.models.iter().map(|model| ModelOption {
                profile_name: profile.name.clone(),
                model: model.id.clone(),
                provider_kind: profile.provider_kind.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelConfig {
        ModelConfig::new(id)
    }

    fn profile(name: &str, models: &[&str]) -> Profile {
        Profile {
            name: name.to_string(),
            models: models.iter().map(|item| model(item)).collect(),
            ..Profile::default()
        }
    }

    #[test]
    fn active_selection_falls_back_to_first_profile_with_warning() {
        let profiles = vec![
            profile("deepseek", &["deepseek-chat"]),
            profile("anthropic", &["claude"]),
        ];

        let resolved = resolve_active_selection("missing", "missing-model", &profiles)
            .expect("selection should resolve");

        assert_eq!(resolved.active_profile, "deepseek");
        assert_eq!(resolved.active_model, "deepseek-chat");
        assert!(resolved.warning.is_some());
    }

    #[test]
    fn active_selection_falls_back_to_first_model_with_warning() {
        let profiles = vec![profile("deepseek", &["deepseek-chat", "deepseek-reasoner"])];

        let resolved = resolve_active_selection("deepseek", "missing-model", &profiles)
            .expect("selection should resolve");

        assert_eq!(resolved.active_profile, "deepseek");
        assert_eq!(resolved.active_model, "deepseek-chat");
        assert!(resolved.warning.is_some());
    }

    #[test]
    fn current_model_rejects_profiles_without_models() {
        let config = Config {
            active_profile: "empty".to_string(),
            active_model: "missing-model".to_string(),
            profiles: vec![profile("empty", &[])],
            ..Config::default()
        };

        let error = resolve_current_model(&config).expect_err("empty profile should be rejected");

        assert!(error.to_string().contains("has no models"));
    }
}
