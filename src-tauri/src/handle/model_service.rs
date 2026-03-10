use astrcode_core::config::load_config;
use astrcode_core::{
    open_config_in_editor as open_config_file_in_editor, save_config, test_connection, TestResult,
};

use super::presentation::{build_config_view, list_model_options, resolve_current_model};
use super::{AgentHandle, ConfigView, CurrentModelInfo, ModelOption};

impl AgentHandle {
    pub async fn get_config() -> Result<ConfigView, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        let config_path = astrcode_core::config::config_path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();
        build_config_view(&config, config_path)
    }

    pub async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> Result<(), String> {
        self.set_model(active_profile, active_model).await
    }

    pub async fn set_model(&self, profile_name: String, model: String) -> Result<(), String> {
        let mut config = load_config().map_err(|e| e.to_string())?;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .ok_or_else(|| format!("profile '{}' does not exist", profile_name))?;

        if profile.models.is_empty() {
            return Err(format!("profile '{}' has no models", profile_name));
        }

        if !profile
            .models
            .iter()
            .any(|profile_model| profile_model == &model)
        {
            return Err(format!(
                "model '{}' does not exist in profile '{}'",
                model, profile_name
            ));
        }

        config.active_profile = profile_name;
        config.active_model = model;
        save_config(&config).map_err(|e| e.to_string())
    }

    pub async fn get_current_model(&self) -> Result<CurrentModelInfo, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        resolve_current_model(&config)
    }

    pub async fn list_available_models(&self) -> Result<Vec<ModelOption>, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        Ok(list_model_options(&config))
    }

    pub async fn test_connection_for_selection(
        &self,
        profile_name: String,
        model: String,
    ) -> Result<TestResult, String> {
        let config = load_config().map_err(|e| e.to_string())?;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .ok_or_else(|| format!("profile '{}' does not exist", profile_name))?;

        test_connection(profile, &model)
            .await
            .map_err(|e| e.to_string())
    }

    pub fn open_config_in_editor() -> Result<(), String> {
        open_config_file_in_editor().map_err(|e| e.to_string())
    }
}
