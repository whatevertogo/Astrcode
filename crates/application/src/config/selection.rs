//! 模型选择辅助函数。
//!
//! 封装 Profile 和 Model 的选择与回退逻辑，确保所有调用方都使用同一套解析规则。
//! 选择逻辑会处理以下场景：
//! - active_profile 不存在 → 回退到第一个 profile
//! - active_model 不在 profile 中 → 回退到 profile 的第一个 model
//! - profile 无 model → 返回错误

use astrcode_core::{
    ActiveSelection, Config, CurrentModelSelection, ModelConfig, ModelOption, ModelSelection,
    Profile,
};

use crate::ApplicationError;

/// 解析活跃 profile/model 选择，支持回退到第一个可用项。
///
/// 如果指定的 profile 或 model 不存在，会自动回退并生成警告信息。
pub fn resolve_active_selection(
    active_profile: &str,
    active_model: &str,
    profiles: &[Profile],
) -> Result<ActiveSelection, ApplicationError> {
    if profiles.is_empty() {
        return Err(ApplicationError::InvalidArgument(
            "no profiles configured".to_string(),
        ));
    }

    let selected_profile = profiles
        .iter()
        .find(|p| p.name == active_profile)
        .unwrap_or(&profiles[0]);

    if selected_profile.name != active_profile {
        return fallback_selection(
            selected_profile,
            format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            ),
        );
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|m| m.id == active_model)
    {
        return Ok(active_selection(selected_profile, model.id.clone(), None));
    }

    let fallback_model = first_model_id(selected_profile)?.to_string();
    Ok(active_selection(
        selected_profile,
        fallback_model.clone(),
        Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, fallback_model
        )),
    ))
}

/// 获取当前生效的模型信息。
pub fn resolve_current_model(config: &Config) -> Result<CurrentModelSelection, ApplicationError> {
    let selected = resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )?;

    let profile = config
        .profiles
        .iter()
        .find(|p| p.name == selected.active_profile)
        .ok_or_else(|| {
            ApplicationError::InvalidArgument(format!(
                "active profile '{}' not found",
                selected.active_profile
            ))
        })?;

    Ok(ModelSelection::new(
        selected.active_profile,
        selected.active_model,
        profile.provider_kind.clone(),
    ))
}

/// 解析指定 profile 中匹配的 model，找不到时回退到第一个 model。
pub fn resolve_model_for_profile<'a>(
    profile: &'a Profile,
    active_model: &str,
) -> Result<Option<&'a ModelConfig>, ApplicationError> {
    if profile.models.is_empty() {
        return Err(ApplicationError::InvalidArgument(format!(
            "profile '{}' has no models",
            profile.name
        )));
    }

    Ok(profile
        .models
        .iter()
        .find(|m| m.id == active_model)
        .or_else(|| profile.models.first()))
}

fn first_model_id(profile: &Profile) -> Result<&str, ApplicationError> {
    profile
        .models
        .first()
        .map(|model| model.id.as_str())
        .ok_or_else(|| {
            ApplicationError::InvalidArgument(format!(
                "profile '{}' has no models configured",
                profile.name
            ))
        })
}

fn fallback_selection(
    profile: &Profile,
    warning: String,
) -> Result<ActiveSelection, ApplicationError> {
    Ok(active_selection(
        profile,
        first_model_id(profile)?.to_string(),
        Some(warning),
    ))
}

fn active_selection(
    profile: &Profile,
    active_model: String,
    warning: Option<String>,
) -> ActiveSelection {
    ActiveSelection {
        active_profile: profile.name.clone(),
        active_model,
        warning,
    }
}

/// 列出所有可用的模型选项。
pub fn list_model_options(config: &Config) -> Vec<ModelOption> {
    config
        .profiles
        .iter()
        .flat_map(|profile| {
            profile.models.iter().map(|model| {
                ModelSelection::new(
                    profile.name.clone(),
                    model.id.clone(),
                    profile.provider_kind.clone(),
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use astrcode_core::{ModelConfig, Profile};

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
            profile("openai", &["gpt-4.1"]),
        ];

        let resolved = resolve_active_selection("missing", "missing-model", &profiles)
            .expect("should resolve");

        assert_eq!(resolved.active_profile, "deepseek");
        assert_eq!(resolved.active_model, "deepseek-chat");
        assert!(resolved.warning.is_some());
    }

    #[test]
    fn active_selection_falls_back_to_first_model_with_warning() {
        let profiles = vec![profile("deepseek", &["deepseek-chat", "deepseek-reasoner"])];

        let resolved = resolve_active_selection("deepseek", "missing-model", &profiles)
            .expect("should resolve");

        assert_eq!(resolved.active_profile, "deepseek");
        assert_eq!(resolved.active_model, "deepseek-chat");
        assert!(resolved.warning.is_some());
    }

    #[test]
    fn active_selection_exact_match_has_no_warning() {
        let profiles = vec![profile("deepseek", &["deepseek-chat", "deepseek-reasoner"])];

        let resolved = resolve_active_selection("deepseek", "deepseek-reasoner", &profiles)
            .expect("should resolve");

        assert_eq!(resolved.active_profile, "deepseek");
        assert_eq!(resolved.active_model, "deepseek-reasoner");
        assert!(resolved.warning.is_none());
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

    #[test]
    fn list_model_options_flattens_all_profiles() {
        let profiles = vec![
            profile("deepseek", &["deepseek-chat"]),
            profile("openai", &["gpt-4.1", "gpt-4.1-mini"]),
        ];
        let config = Config {
            profiles,
            ..Config::default()
        };

        let options = list_model_options(&config);
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].model, "deepseek-chat");
        assert_eq!(options[1].model, "gpt-4.1");
        assert_eq!(options[2].model, "gpt-4.1-mini");
    }
}
