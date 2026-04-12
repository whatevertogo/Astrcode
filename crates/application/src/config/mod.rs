use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_core::{
    ActiveSelection, Config, CurrentModelSelection, ModelOption, Profile, home::resolve_home_dir,
};
use tokio::sync::RwLock;

use crate::ApplicationError;

/// 模型连通性测试结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestConnectionResult {
    pub success: bool,
    pub provider: String,
    pub model: String,
    pub error: Option<String>,
}

/// 应用配置服务：负责配置 IO、活跃选择、默认值与校验。
#[derive(Debug, Clone)]
pub struct ConfigService {
    path: PathBuf,
    config: Arc<RwLock<Config>>,
}

impl Default for ConfigService {
    fn default() -> Self {
        let path = default_config_path();
        let initial = load_config_from_path(&path).unwrap_or_default();
        Self {
            path,
            config: Arc::new(RwLock::new(initial)),
        }
    }
}

impl ConfigService {
    pub fn new(path: PathBuf) -> Self {
        let initial = load_config_from_path(&path).unwrap_or_default();
        Self {
            path,
            config: Arc::new(RwLock::new(initial)),
        }
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn current_config_path(&self) -> PathBuf {
        self.path.clone()
    }

    pub async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> Result<(), ApplicationError> {
        let mut config = self.config.write().await;
        let selection = resolve_active_selection(&active_profile, &active_model, &config.profiles)?;
        config.active_profile = selection.active_profile;
        config.active_model = selection.active_model;
        persist_config_to_path(&self.path, &config)?;
        Ok(())
    }

    pub async fn reload_config_from_disk(&self) -> Result<Config, ApplicationError> {
        let loaded = load_config_from_path(&self.path)?;
        let mut guard = self.config.write().await;
        *guard = loaded.clone();
        Ok(loaded)
    }

    pub async fn test_connection(
        &self,
        profile_name: &str,
        model: &str,
    ) -> Result<TestConnectionResult, ApplicationError> {
        let config = self.config.read().await;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .ok_or_else(|| {
                ApplicationError::InvalidArgument(format!("profile '{}' not found", profile_name))
            })?;
        let model_exists = profile.models.iter().any(|item| item.id == model);
        Ok(TestConnectionResult {
            success: model_exists,
            provider: profile.provider_kind.clone(),
            model: model.to_string(),
            error: (!model_exists).then_some(format!(
                "model '{}' not configured under profile '{}'",
                model, profile_name
            )),
        })
    }
}

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
        .find(|profile| profile.name == active_profile)
        .unwrap_or(&profiles[0]);

    let selected_model = selected_profile
        .models
        .iter()
        .find(|model| model.id == active_model)
        .map(|model| model.id.clone())
        .or_else(|| {
            selected_profile
                .models
                .first()
                .map(|model| model.id.clone())
        })
        .ok_or_else(|| {
            ApplicationError::InvalidArgument(format!(
                "profile '{}' has no models configured",
                selected_profile.name
            ))
        })?;

    Ok(ActiveSelection {
        active_profile: selected_profile.name.clone(),
        active_model: selected_model,
        warning: None,
    })
}

pub fn resolve_current_model(config: &Config) -> Result<CurrentModelSelection, ApplicationError> {
    let selected = resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )?;

    let profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == selected.active_profile)
        .ok_or_else(|| {
            ApplicationError::InvalidArgument(format!(
                "active profile '{}' not found",
                selected.active_profile
            ))
        })?;

    Ok(CurrentModelSelection {
        profile_name: selected.active_profile,
        model: selected.active_model,
        provider_kind: profile.provider_kind.clone(),
    })
}

pub fn list_model_options(config: &Config) -> Vec<ModelOption> {
    let mut options = Vec::new();
    for profile in &config.profiles {
        for model in &profile.models {
            options.push(ModelOption {
                profile_name: profile.name.clone(),
                model: model.id.clone(),
                provider_kind: profile.provider_kind.clone(),
            });
        }
    }
    options
}

pub fn is_env_var_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn default_config_path() -> PathBuf {
    resolve_home_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".astrcode")
        .join("config.toml")
}

fn load_config_from_path(path: &Path) -> Result<Config, ApplicationError> {
    if !path.is_file() {
        return Ok(Config::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|error| {
        ApplicationError::Internal(format!(
            "failed to read config '{}': {}",
            path.display(),
            error
        ))
    })?;
    toml::from_str::<Config>(&raw).map_err(|error| {
        ApplicationError::InvalidArgument(format!(
            "failed to parse config '{}': {}",
            path.display(),
            error
        ))
    })
}

fn persist_config_to_path(path: &Path, config: &Config) -> Result<(), ApplicationError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ApplicationError::Internal(format!(
                "failed to create config dir '{}': {}",
                parent.display(),
                error
            ))
        })?;
    }
    let encoded = toml::to_string_pretty(config).map_err(|error| {
        ApplicationError::Internal(format!("failed to serialize config: {error}"))
    })?;
    std::fs::write(path, encoded).map_err(|error| {
        ApplicationError::Internal(format!(
            "failed to write config '{}': {}",
            path.display(),
            error
        ))
    })
}
