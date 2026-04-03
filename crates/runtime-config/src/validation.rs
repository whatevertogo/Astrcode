//! 配置规范化、迁移与验证。
//!
//! 本模块确保加载和保存的配置始终处于合法状态：
//! - **迁移**：空白 version 字段迁移到当前版本，空白 active_profile/active_model 填充默认值
//! - **验证**：检查 Provider 类型合法性、Profile 名称唯一性、模型列表非空、
//!   active_profile/active_model 交叉引用合法性、运行时参数范围
//!
//! # 验证失败策略
//!
//! 验证失败返回 `AstrError::Validation` 错误，包含具体的字段名称和原因。
//! 错误信息会被传播到 HTTP 响应中，供前端展示给用户。

use std::collections::HashSet;

use astrcode_core::{AstrError, Result};

use crate::constants::{CURRENT_CONFIG_VERSION, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI};
use crate::types::Config;

/// 规范化并验证配置。
///
/// 依次执行迁移（[`migrate_config`]）和验证（[`validate_config`]），
/// 确保配置处于合法且最新的状态。
pub fn normalize_config(mut config: Config) -> Result<Config> {
    migrate_config(&mut config)?;
    validate_config(&config)?;
    Ok(config)
}

/// 将配置迁移到当前 schema 版本。
///
/// 处理以下迁移场景：
/// - 空白 `version` → 填充为 [`CURRENT_CONFIG_VERSION`]
/// - 空白 `active_profile` → 填充为默认 Profile 名称
/// - 空白 `active_model` → 填充为默认模型名称
///
/// 不支持的版本号会返回验证错误。
fn migrate_config(config: &mut Config) -> Result<()> {
    if config.version.trim().is_empty() {
        config.version = CURRENT_CONFIG_VERSION.to_string();
    }

    if config.version != CURRENT_CONFIG_VERSION {
        return Err(AstrError::Validation(format!(
            "unsupported config version: {}",
            config.version
        )));
    }

    if config.active_profile.trim().is_empty() {
        config.active_profile = "deepseek".to_string();
    }

    if config.active_model.trim().is_empty() {
        config.active_model = "deepseek-chat".to_string();
    }

    Ok(())
}

/// 验证配置的合法性。
///
/// 检查项包括：
/// - 运行时参数必须大于 0（`max_tool_concurrency`、`tool_result_max_bytes` 等）
/// - `compact_threshold_percent` 必须在 1-100 范围内
/// - 至少包含一个 Profile
/// - Profile 名称不能为空且必须唯一
/// - 每个 Profile 必须至少有一个模型
/// - `max_tokens` 必须大于 0
/// - `provider_kind` 必须是支持的类型（`openai-compatible` 或 `anthropic`）
/// - OpenAI 兼容 Provider 的 `base_url` 不能为空
/// - `active_profile` 必须存在于 `profiles` 列表中
/// - `active_model` 必须存在于 `active_profile` 的 `models` 列表中
pub fn validate_config(config: &Config) -> Result<()> {
    if matches!(config.runtime.max_tool_concurrency, Some(0)) {
        return Err(AstrError::Validation(
            "runtime.maxToolConcurrency must be greater than 0".to_string(),
        ));
    }
    if matches!(config.runtime.tool_result_max_bytes, Some(0)) {
        return Err(AstrError::Validation(
            "runtime.toolResultMaxBytes must be greater than 0".to_string(),
        ));
    }
    if matches!(config.runtime.compact_keep_recent_turns, Some(0)) {
        return Err(AstrError::Validation(
            "runtime.compactKeepRecentTurns must be greater than 0".to_string(),
        ));
    }
    if matches!(config.runtime.continuation_min_delta_tokens, Some(0)) {
        return Err(AstrError::Validation(
            "runtime.continuationMinDeltaTokens must be greater than 0".to_string(),
        ));
    }
    if matches!(config.runtime.max_continuations, Some(0)) {
        return Err(AstrError::Validation(
            "runtime.maxContinuations must be greater than 0".to_string(),
        ));
    }
    if let Some(percent) = config.runtime.compact_threshold_percent {
        if !(1..=100).contains(&percent) {
            return Err(AstrError::Validation(
                "runtime.compactThresholdPercent must be between 1 and 100".to_string(),
            ));
        }
    }

    if config.profiles.is_empty() {
        return Err(AstrError::Validation(
            "config must contain at least one profile".to_string(),
        ));
    }

    let mut seen_names = HashSet::new();
    for profile in &config.profiles {
        if profile.name.trim().is_empty() {
            return Err(AstrError::Validation(
                "profile name cannot be empty".to_string(),
            ));
        }
        if !seen_names.insert(profile.name.as_str()) {
            return Err(AstrError::Validation(format!(
                "duplicate profile name: {}",
                profile.name
            )));
        }
        if profile.models.is_empty() {
            return Err(AstrError::Validation(format!(
                "profile '{}' must contain at least one model",
                profile.name
            )));
        }
        if profile.max_tokens == 0 {
            return Err(AstrError::Validation(format!(
                "profile '{}' max_tokens must be greater than 0",
                profile.name
            )));
        }
        match profile.provider_kind.as_str() {
            PROVIDER_KIND_OPENAI => {
                if profile.base_url.trim().is_empty() {
                    return Err(AstrError::Validation(format!(
                        "profile '{}' base_url cannot be empty",
                        profile.name
                    )));
                }
            }
            PROVIDER_KIND_ANTHROPIC => {}
            other => {
                return Err(AstrError::Validation(format!(
                    "profile '{}' has unsupported provider_kind '{}'",
                    profile.name, other
                )))
            }
        }
    }

    let active_profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == config.active_profile)
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "active_profile '{}' not found",
                config.active_profile
            ))
        })?;
    if !active_profile
        .models
        .iter()
        .any(|model| model == &config.active_model)
    {
        return Err(AstrError::Validation(format!(
            "active_model '{}' is not configured under profile '{}'",
            config.active_model, config.active_profile
        )));
    }

    Ok(())
}
