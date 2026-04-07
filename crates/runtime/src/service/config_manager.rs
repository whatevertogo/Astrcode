use std::{path::PathBuf, sync::Arc};

use super::{
    RuntimeService, ServiceError, ServiceResult, blocking_bridge::spawn_blocking_service,
    loop_factory::LoopRuntimeDeps,
};
use crate::config::{config_path, open_config_in_editor, save_config, test_connection};

/// 配置管理器：负责配置快照读写与磁盘重载。
///
/// 该组件把“配置语义”从 RuntimeService 门面中分离，
/// 后续如果接入远端配置源或多租户配置，可以在这里演进而不污染主服务。
pub(super) struct ConfigManager<'a> {
    runtime: &'a RuntimeService,
}

impl<'a> ConfigManager<'a> {
    pub(super) fn new(runtime: &'a RuntimeService) -> Self {
        Self { runtime }
    }

    pub(super) async fn get_config(&self) -> crate::config::Config {
        self.runtime.config.lock().await.clone()
    }

    pub(super) async fn reload_config_from_disk(&self) -> ServiceResult<crate::config::Config> {
        let next_config = spawn_blocking_service("reload config from disk", || {
            crate::config::load_config().map_err(ServiceError::from)
        })
        .await?;

        let _guard = self.runtime.rebuild_lock.lock().await;
        let surface = self.runtime.surface.read().await.clone();
        let next_loop = super::build_agent_loop(
            &surface,
            &next_config.active_profile,
            &next_config.runtime,
            LoopRuntimeDeps::new(
                Arc::clone(&self.runtime.policy),
                Arc::clone(&self.runtime.approval),
                Some(self.runtime.agent_profile_catalog()),
            ),
        );

        *self.runtime.config.lock().await = next_config.clone();
        *self.runtime.loop_.write().await = next_loop;
        Ok(next_config)
    }

    pub(super) async fn reload_agent_profiles_from_disk(
        &self,
    ) -> ServiceResult<Arc<crate::AgentProfileRegistry>> {
        let loader = self.runtime.agent_loader();
        let next_registry = spawn_blocking_service("reload agent profiles from disk", move || {
            loader.load().map_err(|error| {
                ServiceError::Internal(astrcode_core::AstrError::Validation(error.to_string()))
            })
        })
        .await?;
        let next_registry = Arc::new(next_registry);

        let _guard = self.runtime.rebuild_lock.lock().await;
        *self.runtime.agent_profiles.write().map_err(|_| {
            ServiceError::Internal(astrcode_core::AstrError::LockPoisoned(
                "agent profile registry".to_string(),
            ))
        })? = Arc::clone(&next_registry);
        Ok(next_registry)
    }

    pub(super) async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> ServiceResult<()> {
        let mut config = self.runtime.config.lock().await;
        let profile = config
            .profiles
            .iter()
            .find(|profile| profile.name == active_profile)
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!("profile '{}' does not exist", active_profile))
            })?;

        if !profile.models.iter().any(|model| model.id == active_model) {
            return Err(ServiceError::InvalidInput(format!(
                "model '{}' does not exist in profile '{}'",
                active_model, active_profile
            )));
        }

        config.active_profile = active_profile;
        config.active_model = active_model;
        save_config(&config).map_err(ServiceError::from)
    }

    pub(super) async fn current_config_path(&self) -> ServiceResult<PathBuf> {
        spawn_blocking_service("resolve config path", || {
            config_path().map_err(ServiceError::from)
        })
        .await
    }

    pub(super) async fn open_config_in_editor(&self) -> ServiceResult<()> {
        spawn_blocking_service("open config in editor", || {
            open_config_in_editor().map_err(ServiceError::from)
        })
        .await
    }

    pub(super) async fn test_connection(
        &self,
        profile_name: &str,
        model: &str,
    ) -> ServiceResult<crate::config::TestResult> {
        let config = self.runtime.config.lock().await.clone();
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
