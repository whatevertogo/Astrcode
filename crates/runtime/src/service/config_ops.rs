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

use std::{path::PathBuf, sync::Arc, time::Duration};

use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};

use super::{RuntimeService, ServiceError, ServiceResult, support::spawn_blocking_service};
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

pub(super) async fn run_config_watch_loop(service: Arc<RuntimeService>) -> ServiceResult<()> {
    let watched_config_path = spawn_blocking_service("resolve config watch path", || {
        config_path().map_err(ServiceError::from)
    })
    .await?;
    let watch_dir = watched_config_path
        .parent()
        .ok_or_else(|| {
            ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
                "config path '{}' has no parent directory",
                watched_config_path.display()
            )))
        })?
        .to_path_buf();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            let _ = tx.send(result);
        },
        NotifyConfig::default(),
    )
    .map_err(|error| {
        ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
            "failed to create config watcher: {error}"
        )))
    })?;

    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|error| {
            ServiceError::Internal(astrcode_core::AstrError::Internal(format!(
                "failed to watch config directory '{}': {error}",
                watch_dir.display()
            )))
        })?;

    loop {
        tokio::select! {
            _ = service.shutdown_token.cancelled() => return Ok(()),
            maybe_event = rx.recv() => {
                let Some(result) = maybe_event else {
                    return Ok(());
                };

                match result {
                    Ok(event) => {
                        if !event_targets_config(&event, &watched_config_path) {
                            continue;
                        }
                    }
                    Err(error) => {
                        log::warn!("config watcher delivered an error: {}", error);
                        continue;
                    }
                }

                let debounce = tokio::time::sleep(Duration::from_millis(300));
                tokio::pin!(debounce);
                loop {
                    tokio::select! {
                        _ = service.shutdown_token.cancelled() => return Ok(()),
                        _ = &mut debounce => break,
                        maybe_next = rx.recv() => {
                            let Some(next) = maybe_next else {
                                return Ok(());
                            };
                            if let Err(error) = next {
                                log::warn!("config watcher delivered an error: {}", error);
                            }
                        }
                    }
                }

                match service.reload_config_from_disk().await {
                    Ok(config) => {
                        log::info!(
                            "reloaded config from disk: active_profile='{}', active_model='{}'",
                            config.active_profile,
                            config.active_model
                        );
                    }
                    Err(error) => {
                        log::warn!("failed to hot-reload config from disk: {}", error);
                    }
                }
            }
        }
    }
}

fn event_targets_config(event: &Event, config_path: &std::path::Path) -> bool {
    let Some(config_file_name) = config_path.file_name() else {
        return false;
    };

    event.paths.iter().any(|path| {
        path == config_path
            || path
                .file_name()
                .is_some_and(|file_name| file_name == config_file_name)
    })
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
}
