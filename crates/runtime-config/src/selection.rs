//! Config selection helpers.
//!
//! Keep fallback selection rules here so every surface resolves config the same
//! way instead of teaching HTTP routes their own recovery logic.

use anyhow::{anyhow, Result};

use crate::{Config, Profile};

/// Resolved active selection after applying profile/model fallbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSelection {
    pub active_profile: String,
    pub active_model: String,
    pub warning: Option<String>,
}

/// The effective model the runtime will use for the current config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentModelSelection {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

/// A flattened model option that callers can project into their own DTOs.
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
        .ok_or_else(|| anyhow!("no profiles configured"))?;

    let selected_profile = profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .unwrap_or(fallback_profile);

    if selected_profile.models.is_empty() {
        return Err(anyhow!("profile '{}' has no models", selected_profile.name));
    }

    if selected_profile.name != active_profile {
        return Ok(ActiveSelection {
            active_profile: selected_profile.name.clone(),
            active_model: selected_profile.models[0].clone(),
            warning: Some(format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            )),
        });
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|model| *model == active_model)
    {
        return Ok(ActiveSelection {
            active_profile: selected_profile.name.clone(),
            active_model: model.clone(),
            warning: None,
        });
    }

    Ok(ActiveSelection {
        active_profile: selected_profile.name.clone(),
        active_model: selected_profile.models[0].clone(),
        warning: Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, selected_profile.models[0]
        )),
    })
}

pub fn resolve_current_model(config: &Config) -> Result<CurrentModelSelection> {
    let profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == config.active_profile)
        .or_else(|| config.profiles.first())
        .ok_or_else(|| anyhow!("no profiles configured"))?;

    let model = if profile
        .models
        .iter()
        .any(|item| item == &config.active_model)
    {
        config.active_model.clone()
    } else {
        profile
            .models
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("profile '{}' has no models", profile.name))?
    };

    Ok(CurrentModelSelection {
        profile_name: profile.name.clone(),
        model,
        provider_kind: profile.provider_kind.clone(),
    })
}

pub fn list_model_options(config: &Config) -> Vec<ModelOption> {
    config
        .profiles
        .iter()
        .flat_map(|profile| {
            profile.models.iter().map(|model| ModelOption {
                profile_name: profile.name.clone(),
                model: model.clone(),
                provider_kind: profile.provider_kind.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, models: &[&str]) -> Profile {
        Profile {
            name: name.to_string(),
            models: models.iter().map(|model| (*model).to_string()).collect(),
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
    fn current_model_falls_back_to_first_profile_and_model() {
        let config = Config {
            active_profile: "missing".to_string(),
            active_model: "missing-model".to_string(),
            profiles: vec![profile("deepseek", &["deepseek-chat"])],
            ..Config::default()
        };

        let resolved = resolve_current_model(&config).expect("current model should resolve");

        assert_eq!(resolved.profile_name, "deepseek");
        assert_eq!(resolved.model, "deepseek-chat");
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
    fn model_options_flatten_profiles() {
        let config = Config {
            profiles: vec![
                profile("deepseek", &["deepseek-chat"]),
                profile("anthropic", &["claude"]),
            ],
            ..Config::default()
        };

        let options = list_model_options(&config);

        assert_eq!(options.len(), 2);
        assert_eq!(options[0].profile_name, "deepseek");
        assert_eq!(options[1].profile_name, "anthropic");
    }
}
