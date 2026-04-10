use std::{path::PathBuf, sync::Arc};

use crate::service::{RuntimeService, ServiceResult};

mod service;

/// `runtime-config` 的唯一 surface handle。
#[derive(Clone)]
pub struct ConfigServiceHandle {
    runtime: Arc<RuntimeService>,
}

impl ConfigServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    fn service(&self) -> service::ConfigService<'_> {
        service::ConfigService::new(self.runtime.as_ref())
    }

    pub async fn get_config(&self) -> crate::config::Config {
        self.service().get_config().await
    }

    pub async fn reload_config_from_disk(&self) -> ServiceResult<crate::config::Config> {
        self.service().reload_config_from_disk().await
    }

    pub async fn reload_agent_profiles_from_disk(
        &self,
    ) -> ServiceResult<Arc<crate::AgentProfileRegistry>> {
        self.service().reload_agent_profiles_from_disk().await
    }

    pub async fn save_active_selection(
        &self,
        active_profile: String,
        active_model: String,
    ) -> ServiceResult<()> {
        self.service()
            .save_active_selection(active_profile, active_model)
            .await
    }

    pub async fn current_config_path(&self) -> ServiceResult<PathBuf> {
        self.service().current_config_path().await
    }

    pub async fn open_config_in_editor(&self) -> ServiceResult<()> {
        self.service().open_config_in_editor().await
    }

    pub async fn test_connection(
        &self,
        profile_name: &str,
        model: &str,
    ) -> ServiceResult<crate::config::TestResult> {
        self.service().test_connection(profile_name, model).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, ModelConfig, Profile, RuntimeConfig, save_config},
        service::ServiceError,
        test_support::{TestEnvGuard, empty_capabilities},
    };

    #[tokio::test]
    async fn save_active_selection_rejects_missing_profile() {
        let _guard = TestEnvGuard::new();
        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));

        let err = service
            .config()
            .save_active_selection("missing".to_string(), "model-a".to_string())
            .await
            .expect_err("missing profile should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn save_active_selection_rejects_missing_model() {
        let _guard = TestEnvGuard::new();
        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));
        {
            let mut config = service.config.lock().await;
            *config = Config {
                active_profile: "custom".to_string(),
                active_model: "model-a".to_string(),
                profiles: vec![Profile {
                    name: "custom".to_string(),
                    models: vec![ModelConfig {
                        id: "model-a".to_string(),
                        max_tokens: Some(8096),
                        context_limit: Some(128_000),
                    }],
                    api_key: Some("TEST_API_KEY".to_string()),
                    ..Profile::default()
                }],
                ..Config::default()
            };
        }

        let err = service
            .config()
            .save_active_selection("custom".to_string(), "missing-model".to_string())
            .await
            .expect_err("missing model should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("does not exist in profile"));
    }

    #[tokio::test]
    async fn service_uses_runtime_max_tool_concurrency_from_config_file() {
        let _guard = TestEnvGuard::new();
        save_config(&Config {
            runtime: RuntimeConfig {
                max_tool_concurrency: Some(6),
                ..RuntimeConfig::default()
            },
            ..Config::default()
        })
        .expect("config should save");

        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));
        let loop_ = service.loop_surface().current_loop().await;

        assert_eq!(loop_.max_tool_concurrency(), 6);
    }

    #[tokio::test]
    async fn service_uses_runtime_compact_keep_recent_turns_from_config_file() {
        let _guard = TestEnvGuard::new();
        save_config(&Config {
            runtime: RuntimeConfig {
                compact_keep_recent_turns: Some(2),
                ..RuntimeConfig::default()
            },
            ..Config::default()
        })
        .expect("config should save");

        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));
        let loop_ = service.loop_surface().current_loop().await;

        assert_eq!(loop_.compact_keep_recent_turns(), 2);
    }

    #[tokio::test]
    async fn service_uses_default_compact_keep_recent_turns_when_runtime_value_is_missing() {
        let _guard = TestEnvGuard::new();
        save_config(&Config {
            runtime: RuntimeConfig::default(),
            ..Config::default()
        })
        .expect("config should save");

        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));
        let loop_ = service.loop_surface().current_loop().await;

        assert_eq!(
            loop_.compact_keep_recent_turns(),
            crate::config::DEFAULT_COMPACT_KEEP_RECENT_TURNS as usize
        );
    }

    #[tokio::test]
    async fn reload_config_from_disk_rebuilds_loop_with_new_runtime_settings() {
        let _guard = TestEnvGuard::new();
        save_config(&Config {
            runtime: RuntimeConfig {
                max_tool_concurrency: Some(2),
                ..RuntimeConfig::default()
            },
            ..Config::default()
        })
        .expect("initial config should save");

        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));
        assert_eq!(
            service
                .loop_surface()
                .current_loop()
                .await
                .max_tool_concurrency(),
            2
        );

        save_config(&Config {
            active_profile: "deepseek".to_string(),
            active_model: "deepseek-reasoner".to_string(),
            runtime: RuntimeConfig {
                max_tool_concurrency: Some(7),
                ..RuntimeConfig::default()
            },
            ..Config::default()
        })
        .expect("updated config should save");

        let reloaded = service
            .config()
            .reload_config_from_disk()
            .await
            .expect("reload should succeed");

        assert_eq!(reloaded.active_model, "deepseek-reasoner");
        assert_eq!(
            service.config().get_config().await.active_model,
            "deepseek-reasoner"
        );
        assert_eq!(
            service
                .loop_surface()
                .current_loop()
                .await
                .max_tool_concurrency(),
            7
        );
    }

    #[tokio::test]
    async fn reload_agent_profiles_from_disk_replaces_registry_snapshot() {
        let guard = TestEnvGuard::new();
        let agents_dir = guard.home_dir().join(".astrcode").join("agents");
        std::fs::create_dir_all(&agents_dir).expect("agents dir should be created");
        std::fs::write(
            agents_dir.join("review.md"),
            r#"---
name: review
description: 初始审查员
tools: [readFile]
---
先看现状。
"#,
        )
        .expect("initial agent should be written");

        let service =
            Arc::new(RuntimeService::from_capabilities(empty_capabilities()).expect("service"));
        let initial = service
            .config()
            .reload_agent_profiles_from_disk()
            .await
            .expect("initial reload should succeed");
        assert_eq!(
            initial
                .get("review")
                .expect("review profile should exist")
                .description,
            "初始审查员"
        );

        std::fs::write(
            agents_dir.join("review.md"),
            r#"---
name: review
description: 更新后的审查员
tools: [readFile, grep]
---
更新后的提示。
"#,
        )
        .expect("updated agent should be written");

        let reloaded = service
            .config()
            .reload_agent_profiles_from_disk()
            .await
            .expect("reload should succeed");
        let review = reloaded.get("review").expect("review profile should exist");
        assert_eq!(review.description, "更新后的审查员");
        assert_eq!(
            service
                .agent_profiles()
                .get("review")
                .expect("service snapshot should be updated")
                .allowed_tools,
            vec!["readFile".to_string(), "grep".to_string()]
        );
    }
}
