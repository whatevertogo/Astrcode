//! # 配置操作 (Configuration Operations)
//!
//! 提供运行时配置的读取、修改和验证能力，包括：
//! - 获取当前配置快照
//! - 保存活跃配置选择（profile 和 model）
//! - 在编辑器中打开配置文件
//! - 测试 LLM 连接
//!
//! ## 设计
//!
//! 配置操作通过 `RuntimeService` 的 impl 块实现，直接操作内部的 `config` 锁。
//! 涉及磁盘 I/O 的操作（如解析配置路径、打开编辑器）通过 `spawn_blocking_service`
//! 桥接到阻塞线程池，避免阻塞异步运行时。

use std::{path::PathBuf, sync::Arc};

use super::{RuntimeService, ServiceError, ServiceResult, blocking_bridge::spawn_blocking_service};
use crate::config::{config_path, open_config_in_editor, save_config, test_connection};

impl RuntimeService {
    /// 获取当前运行时配置的完整快照。
    ///
    /// 返回配置的克隆副本，调用方可以安全地读取而不持有锁。
    pub async fn get_config(&self) -> crate::config::Config {
        self.config.lock().await.clone()
    }

    /// 从磁盘重新加载用户级配置，并原子替换当前运行时的配置快照与 loop。
    ///
    /// 这条路径只更新运行时“配置维度”的行为参数，例如默认 profile/model、
    /// max tool concurrency、自动压缩阈值等；当前 capability surface 保持不变。
    pub async fn reload_config_from_disk(&self) -> ServiceResult<crate::config::Config> {
        let next_config = spawn_blocking_service("reload config from disk", || {
            crate::config::load_config().map_err(ServiceError::from)
        })
        .await?;

        let _guard = self.rebuild_lock.lock().await;
        let surface = self.surface.read().await.clone();
        let next_loop = super::build_agent_loop(
            &surface,
            &next_config.runtime,
            Arc::clone(&self.policy),
            Arc::clone(&self.approval),
        );

        *self.config.lock().await = next_config.clone();
        *self.loop_.write().await = next_loop;
        Ok(next_config)
    }

    /// 从磁盘重新加载 agent 定义，并原子替换当前 profile 快照。
    ///
    /// 这条路径不重建 agent loop，因为 agent 定义当前只影响子 Agent 的选择与约束，
    /// 不影响主 loop 的 capability surface。
    pub async fn reload_agent_profiles_from_disk(
        &self,
    ) -> ServiceResult<Arc<crate::AgentProfileRegistry>> {
        let loader = self.agent_loader();
        let next_registry = spawn_blocking_service("reload agent profiles from disk", move || {
            loader.load().map_err(|error| {
                ServiceError::Internal(astrcode_core::AstrError::Validation(error.to_string()))
            })
        })
        .await?;
        let next_registry = Arc::new(next_registry);

        let _guard = self.rebuild_lock.lock().await;
        *self.agent_profiles.write().map_err(|_| {
            ServiceError::Internal(astrcode_core::AstrError::LockPoisoned(
                "agent profile registry".to_string(),
            ))
        })? = Arc::clone(&next_registry);
        Ok(next_registry)
    }

    /// 保存活跃的配置选择（profile 和 model）。
    ///
    /// 验证指定的 profile 和 model 是否存在于当前配置中，
    /// 验证通过后更新活跃选择并持久化到磁盘。
    ///
    /// # 错误
    ///
    /// - 如果 profile 不存在，返回 `InvalidInput`
    /// - 如果 model 不属于指定的 profile，返回 `InvalidInput`
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

    /// 解析并返回当前配置文件的绝对路径。
    ///
    /// 此操作涉及文件系统查询，通过阻塞线程池执行。
    pub async fn current_config_path(&self) -> ServiceResult<PathBuf> {
        spawn_blocking_service("resolve config path", || {
            config_path().map_err(ServiceError::from)
        })
        .await
    }

    /// 在系统默认编辑器中打开配置文件。
    ///
    /// 此操作涉及进程启动，通过阻塞线程池执行。
    pub async fn open_config_in_editor(&self) -> ServiceResult<()> {
        spawn_blocking_service("open config in editor", || {
            open_config_in_editor().map_err(ServiceError::from)
        })
        .await
    }

    /// 测试指定 profile 和 model 的 LLM 连接。
    ///
    /// 克隆当前配置以避免长时间持有锁，然后执行连接测试。
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
    use super::*;
    use crate::{
        config::{Config, ModelConfig, Profile, RuntimeConfig, save_config},
        test_support::{TestEnvGuard, empty_capabilities},
    };

    /// 验证保存活跃选择时，不存在的 profile 会被拒绝。
    #[tokio::test]
    async fn save_active_selection_rejects_missing_profile() {
        let _guard = TestEnvGuard::new();
        let service = RuntimeService::from_capabilities(empty_capabilities()).expect("service");

        let err = service
            .save_active_selection("missing".to_string(), "model-a".to_string())
            .await
            .expect_err("missing profile should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("does not exist"));
    }

    /// 验证保存活跃选择时，不属于指定 profile 的 model 会被拒绝。
    #[tokio::test]
    async fn save_active_selection_rejects_missing_model() {
        let _guard = TestEnvGuard::new();
        let service = RuntimeService::from_capabilities(empty_capabilities()).expect("service");
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
            .save_active_selection("custom".to_string(), "missing-model".to_string())
            .await
            .expect_err("missing model should fail");

        assert!(matches!(err, ServiceError::InvalidInput(_)));
        assert!(err.to_string().contains("does not exist in profile"));
    }

    /// 验证运行时从配置文件读取 `max_tool_concurrency` 并应用到 agent loop。
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

        let service = RuntimeService::from_capabilities(empty_capabilities()).expect("service");
        let loop_ = service.current_loop().await;

        assert_eq!(loop_.max_tool_concurrency(), 6);
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

        let service = RuntimeService::from_capabilities(empty_capabilities()).expect("service");
        assert_eq!(service.current_loop().await.max_tool_concurrency(), 2);

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
            .reload_config_from_disk()
            .await
            .expect("reload should succeed");

        assert_eq!(reloaded.active_model, "deepseek-reasoner");
        assert_eq!(service.get_config().await.active_model, "deepseek-reasoner");
        assert_eq!(service.current_loop().await.max_tool_concurrency(), 7);
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

        let service = RuntimeService::from_capabilities(empty_capabilities()).expect("service");
        let initial = service
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
