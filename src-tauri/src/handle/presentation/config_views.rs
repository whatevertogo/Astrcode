use serde::Serialize;

use astrcode_core::config::{Config, Profile};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub name: String,
    pub base_url: String,
    pub api_key_preview: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub config_path: String,
    pub active_profile: String,
    pub active_model: String,
    pub profiles: Vec<ProfileView>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentModelInfo {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

pub(crate) fn build_config_view(config: &Config, config_path: String) -> Result<ConfigView, String> {
    if config.profiles.is_empty() {
        return Ok(ConfigView {
            config_path,
            active_profile: String::new(),
            active_model: String::new(),
            profiles: Vec::new(),
            warning: Some("no profiles configured".to_string()),
        });
    }

    let profiles = config
        .profiles
        .iter()
        .map(|profile| ProfileView {
            name: profile.name.clone(),
            base_url: profile.base_url.clone(),
            api_key_preview: api_key_preview(profile.api_key.as_deref()),
            models: profile.models.clone(),
        })
        .collect::<Vec<_>>();

    let (active_profile, active_model, warning) =
        resolve_active_selection(&config.active_profile, &config.active_model, &config.profiles)?;

    Ok(ConfigView {
        config_path,
        active_profile,
        active_model,
        profiles,
        warning,
    })
}

pub(crate) fn resolve_current_model(config: &Config) -> Result<CurrentModelInfo, String> {
    let profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == config.active_profile)
        .or_else(|| config.profiles.first())
        .ok_or_else(|| "no profiles configured".to_string())?;

    let model = if profile.models.iter().any(|item| item == &config.active_model) {
        config.active_model.clone()
    } else {
        profile
            .models
            .first()
            .cloned()
            .ok_or_else(|| format!("profile '{}' has no models", profile.name))?
    };

    Ok(CurrentModelInfo {
        profile_name: profile.name.clone(),
        model,
        provider_kind: profile.provider_kind.clone(),
    })
}

pub(crate) fn list_model_options(config: &Config) -> Vec<ModelOption> {
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

fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}

fn api_key_preview(api_key: Option<&str>) -> String {
    match api_key.map(str::trim) {
        None => "未配置".to_string(),
        Some("") => "未配置".to_string(),
        Some(value) if is_env_var_name(value) => format!("环境变量: {}", value),
        Some(value) if value.chars().count() > 4 => {
            let suffix = value
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            format!("****{}", suffix)
        }
        Some(_) => "****".to_string(),
    }
}

pub(crate) fn resolve_active_selection(
    active_profile: &str,
    active_model: &str,
    profiles: &[Profile],
) -> Result<(String, String, Option<String>), String> {
    let fallback_profile = profiles
        .first()
        .ok_or_else(|| "no profiles configured".to_string())?;

    let selected_profile = profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .unwrap_or(fallback_profile);

    if selected_profile.models.is_empty() {
        return Err(format!("profile '{}' has no models", selected_profile.name));
    }

    if selected_profile.name != active_profile {
        return Ok((
            selected_profile.name.clone(),
            selected_profile.models[0].clone(),
            Some(format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            )),
        ));
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|model| *model == active_model)
    {
        return Ok((selected_profile.name.clone(), model.clone(), None));
    }

    let fallback_model = selected_profile
        .models
        .first()
        .cloned()
        .ok_or_else(|| format!("profile '{}' has no models", selected_profile.name))?;

    Ok((
        selected_profile.name.clone(),
        fallback_model.clone(),
        Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, fallback_model
        )),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_preview_masks_values_and_env_names() {
        assert_eq!(api_key_preview(None), "未配置");
        assert_eq!(
            api_key_preview(Some("DEEPSEEK_API_KEY")),
            "环境变量: DEEPSEEK_API_KEY"
        );
        assert_eq!(api_key_preview(Some("abcd")), "****");
        assert_eq!(api_key_preview(Some("secret-1234")), "****1234");
    }

    #[test]
    fn resolve_active_selection_falls_back_and_returns_warning() {
        let profiles = vec![Profile {
            name: "default".to_string(),
            models: vec!["model-a".to_string(), "model-b".to_string()],
            ..Profile::default()
        }];

        let (profile, model, warning) =
            resolve_active_selection("missing", "model-z", &profiles).expect("fallback should work");

        assert_eq!(profile, "default");
        assert_eq!(model, "model-a");
        assert_eq!(
            warning.as_deref(),
            Some("配置中的 Profile 不存在，已自动选择 default")
        );
    }
}
