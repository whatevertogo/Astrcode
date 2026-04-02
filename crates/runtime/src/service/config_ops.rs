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

use std::path::PathBuf;

use crate::config::{config_path, open_config_in_editor, save_config, test_connection};

use super::support::spawn_blocking_service;
use super::{RuntimeService, ServiceError, ServiceResult};

impl RuntimeService {
    /// 获取当前运行时配置的完整快照。
    ///
    /// 返回配置的克隆副本，调用方可以安全地读取而不持有锁。
    pub async fn get_config(&self) -> crate::config::Config {
        self.config.lock().await.clone()
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
    use crate::config::{save_config, Config, Profile, RuntimeConfig};
    use crate::test_support::{empty_capabilities, TestEnvGuard};

    use super::*;

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
}
