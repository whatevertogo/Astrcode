use std::path::PathBuf;

use crate::config::{config_path, open_config_in_editor, save_config, test_connection};

use super::support::spawn_blocking_service;
use super::{RuntimeService, ServiceError, ServiceResult};

impl RuntimeService {
    pub async fn get_config(&self) -> crate::config::Config {
        self.config.lock().await.clone()
    }

    pub async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> ServiceResult<()> {
        let mut config = self.config.lock().await;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == active_profile)
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!("profile '{}' does not exist", active_profile))
            })?;

        if !profile.models.iter().any(|model| model == &active_model) {
            return Err(ServiceError::InvalidInput(format!(
                "model '{}' does not exist in profile '{}'",
                active_model, active_profile
            )));
        }

        config.active_profile = active_profile;
        config.active_model = active_model;
        save_config(&config).map_err(ServiceError::from)
    }

    pub async fn current_config_path(&self) -> ServiceResult<PathBuf> {
        spawn_blocking_service("resolve config path", || {
            config_path().map_err(ServiceError::from)
        })
        .await
    }

    pub async fn open_config_in_editor(&self) -> ServiceResult<()> {
        spawn_blocking_service("open config in editor", || {
            open_config_in_editor().map_err(ServiceError::from)
        })
        .await
    }

    pub async fn test_connection(
        &self,
        profile_name: &str,
        model: &str,
    ) -> ServiceResult<crate::config::TestResult> {
        let config = self.config.lock().await.clone();
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!("profile '{}' does not exist", profile_name))
            })?;
        test_connection(profile, model)
            .await
            .map_err(ServiceError::from)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{Config, Profile};
    use crate::test_support::TestEnvGuard;
    use astrcode_core::ToolRegistry;

    use super::*;

    #[tokio::test]
    async fn save_active_selection_rejects_missing_profile() {
        let _guard = TestEnvGuard::new();
        let service = RuntimeService::new(ToolRegistry::builder().build()).expect("service");

        let err = service
            .save_active_selection("missing".to_string(), "model-a".to_string())
            .await
            .expect_err("missing profile should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn save_active_selection_rejects_missing_model() {
        let _guard = TestEnvGuard::new();
        let service = RuntimeService::new(ToolRegistry::builder().build()).expect("service");
        {
            let mut config = service.config.lock().await;
            *config = Config {
                active_profile: "custom".to_string(),
                active_model: "model-a".to_string(),
                profiles: vec![Profile {
                    name: "custom".to_string(),
                    models: vec!["model-a".to_string()],
                    api_key: Some("TEST_API_KEY".to_string()),
                    ..Profile::default()
                }],
                ..Config::default()
            };
        }

        let err = service
            .save_active_selection("custom".to_string(), "missing-model".to_string())
            .await
            .expect_err("missing model should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("does not exist in profile"));
    }
}
