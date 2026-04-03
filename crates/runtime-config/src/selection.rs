//! 配置选择辅助函数。
//!
//! 本模块封装了 Profile 和 Model 的选择与回退逻辑，确保所有调用方（HTTP 路由、
//! CLI、Tauri 命令）都以相同的方式解析配置，而不是各自实现独立的恢复逻辑。
//!
//! # 选择策略
//!
//! 1. 优先使用配置中的 `active_profile` 和 `active_model`
//! 2. 若 `active_profile` 不存在，回退到第一个 Profile 并产生警告
//! 3. 若 `active_model` 不在当前 Profile 的模型列表中，回退到第一个模型并产生警告
//! 4. 若 Profile 的模型列表为空，返回错误
//!
//! 回退逻辑集中在此处，避免 HTTP 路由层各自实现不同的恢复策略。

use astrcode_core::{AstrError, Result};

use crate::{Config, Profile};

/// 应用 Profile/Model 回退后的最终选择结果。
///
/// 包含解析后的活跃 Profile 名称、模型名称，以及可能的警告信息。
/// 警告信息用于告知调用方发生了自动回退（如配置的 Profile 不存在）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSelection {
    pub active_profile: String,
    pub active_model: String,
    pub warning: Option<String>,
}

/// 运行时当前将使用的有效模型信息。
///
/// 与 [`ActiveSelection`] 不同，此类型额外包含 `provider_kind` 字段，
/// 供需要知道 API 协议格式的调用方使用（如 `runtime-llm` crate）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentModelSelection {
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

/// 扁平化的模型选项。
///
/// 将 Profile 和模型的嵌套关系展平为单层列表，供调用方投影到各自的 DTO 中。
/// 典型用途：前端下拉菜单展示所有可用的 Profile-Model 组合。
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

    if selected_profile.models.is_empty() {
        return Err(AstrError::Internal(format!(
            "profile '{}' has no models",
            selected_profile.name
        )));
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
        .ok_or_else(|| AstrError::Internal("no profiles configured".to_string()))?;

    let model = if profile
        .models
        .iter()
        .any(|item| item == &config.active_model)
    {
        config.active_model.clone()
    } else {
        profile.models.first().cloned().ok_or_else(|| {
            AstrError::Internal(format!("profile '{}' has no models", profile.name))
        })?
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
