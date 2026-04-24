//! 配置规范化、迁移与验证。
//!
//! 确保加载和保存的配置始终处于合法状态：
//! - 迁移：空白字段填充默认值
//! - 验证：Provider 类型、Profile 唯一性、模型列表、运行时参数范围

use std::collections::HashSet;

use astrcode_core::{AstrError, Config, ModelConfig, OpenAiApiMode, Result};

use super::constants::PROVIDER_KIND_OPENAI;

macro_rules! validate_positive_fields {
    ($($value:expr => $field:expr),* $(,)?) => {{
        $(validate_positive($value, $field)?;)*
        Ok::<(), astrcode_core::AstrError>(())
    }};
}

/// 规范化并验证配置。
///
/// 依次执行迁移和验证，确保配置合法且处于最新状态。
pub fn normalize_config(mut config: Config) -> Result<Config> {
    migrate_config(&mut config)?;
    validate_config(&config)?;
    Ok(config)
}

/// 将配置迁移到当前 schema 版本。
///
/// 处理空白 version/active_profile/active_model 字段的填充。
fn migrate_config(config: &mut Config) -> Result<()> {
    if config.version.trim().is_empty() {
        config.version = "1".to_string();
    }

    if config.version != "1" {
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
/// 检查项：运行时参数范围、Profile 名称唯一性、模型列表非空、
/// Provider 类型合法性、active_profile/active_model 交叉引用。
pub fn validate_config(config: &Config) -> Result<()> {
    validate_runtime_params(&config.runtime)?;
    validate_profiles(&config.profiles)?;
    validate_active_references(config)?;
    Ok(())
}

fn validate_runtime_params(runtime: &astrcode_core::RuntimeConfig) -> Result<()> {
    validate_positive_fields!(
        runtime.max_tool_concurrency => "runtime.maxToolConcurrency",
        runtime.tool_result_max_bytes => "runtime.toolResultMaxBytes",
        runtime.max_tracked_files => "runtime.maxTrackedFiles",
        runtime.max_recovered_files => "runtime.maxRecoveredFiles",
        runtime.recovery_token_budget => "runtime.recoveryTokenBudget",
        runtime.tool_result_inline_limit => "runtime.toolResultInlineLimit",
        runtime.tool_result_preview_limit => "runtime.toolResultPreviewLimit",
        runtime.max_image_size => "runtime.maxImageSize",
        runtime.max_grep_lines => "runtime.maxGrepLines",
        runtime.session_broadcast_capacity => "runtime.sessionBroadcastCapacity",
        runtime.session_recent_record_limit => "runtime.sessionRecentRecordLimit",
        runtime.max_concurrent_branch_depth => "runtime.maxConcurrentBranchDepth",
        runtime.aggregate_result_bytes_budget => "runtime.aggregateResultBytesBudget",
        runtime.micro_compact_keep_recent_results => "runtime.microCompactKeepRecentResults",
        runtime.max_consecutive_failures => "runtime.maxConsecutiveFailures",
        runtime.max_output_continuation_attempts => "runtime.maxOutputContinuationAttempts",
        runtime.recovery_truncate_bytes => "runtime.recoveryTruncateBytes",
        runtime.reserved_context_size => "runtime.reservedContextSize",
    )?;

    validate_positive_fields!(
        runtime.compact_keep_recent_turns => "runtime.compactKeepRecentTurns",
        runtime.compact_max_retry_attempts => "runtime.compactMaxRetryAttempts",
    )?;

    validate_positive_fields!(
        runtime.llm_connect_timeout_secs => "runtime.llmConnectTimeoutSecs",
        runtime.llm_read_timeout_secs => "runtime.llmReadTimeoutSecs",
        runtime.llm_retry_base_delay_ms => "runtime.llmRetryBaseDelayMs",
        runtime.micro_compact_gap_threshold_secs => "runtime.microCompactGapThresholdSecs",
    )?;

    if let Some(percent) = runtime.compact_threshold_percent {
        if !(1..=100).contains(&percent) {
            return Err(AstrError::Validation(
                "runtime.compactThresholdPercent must be between 1 and 100".to_string(),
            ));
        }
    }
    validate_positive(runtime.api_session_ttl_hours, "runtime.apiSessionTtlHours")?;

    if let Some(agent) = runtime.agent.as_ref() {
        validate_positive_fields!(
            agent.max_subrun_depth => "runtime.agent.maxSubrunDepth",
            agent.max_spawn_per_turn => "runtime.agent.maxSpawnPerTurn",
            agent.max_concurrent => "runtime.agent.maxConcurrent",
            agent.finalized_retain_limit => "runtime.agent.finalizedRetainLimit",
            agent.inbox_capacity => "runtime.agent.inboxCapacity",
            agent.parent_delivery_capacity => "runtime.agent.parentDeliveryCapacity",
        )?;
    }
    Ok(())
}

fn validate_positive<T>(value: Option<T>, field: &str) -> Result<()>
where
    T: Copy + PartialOrd + From<u8>,
{
    if let Some(value) = value {
        if value < T::from(1u8) {
            return Err(AstrError::Validation(format!(
                "{field} must be greater than 0"
            )));
        }
    }
    Ok(())
}

fn validate_profiles(profiles: &[astrcode_core::Profile]) -> Result<()> {
    if profiles.is_empty() {
        return Err(AstrError::Validation(
            "config must contain at least one profile".to_string(),
        ));
    }

    let mut seen_names = HashSet::new();
    for profile in profiles {
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

        let mut seen_model_ids = HashSet::new();
        for model in &profile.models {
            validate_model(profile.name.as_str(), model, &mut seen_model_ids)?;
        }

        if profile.provider_kind.trim().is_empty() {
            return Err(AstrError::Validation(format!(
                "profile '{}' provider_kind cannot be empty",
                profile.name
            )));
        }
        if profile.base_url.trim().is_empty() {
            return Err(AstrError::Validation(format!(
                "profile '{}' base_url cannot be empty",
                profile.name
            )));
        }
        if profile.provider_kind == PROVIDER_KIND_OPENAI {
            for model in &profile.models {
                if model.max_tokens.is_none() || model.context_limit.is_none() {
                    return Err(AstrError::Validation(format!(
                        "openai profile '{}' model '{}' must set both maxTokens and contextLimit",
                        profile.name, model.id
                    )));
                }
            }
            if matches!(profile.api_mode, Some(OpenAiApiMode::Responses))
                && profile.base_url.trim().is_empty()
            {
                return Err(AstrError::Validation(format!(
                    "openai profile '{}' responses mode requires a non-empty baseUrl",
                    profile.name,
                )));
            }
        }
    }
    Ok(())
}

fn validate_active_references(config: &Config) -> Result<()> {
    let active_profile = config
        .profiles
        .iter()
        .find(|p| p.name == config.active_profile)
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "active_profile '{}' not found",
                config.active_profile
            ))
        })?;

    if !active_profile
        .models
        .iter()
        .any(|m| m.id == config.active_model)
    {
        return Err(AstrError::Validation(format!(
            "active_model '{}' is not configured under profile '{}'",
            config.active_model, config.active_profile
        )));
    }

    Ok(())
}

fn validate_model(
    profile_name: &str,
    model: &ModelConfig,
    seen_model_ids: &mut HashSet<String>,
) -> Result<()> {
    if model.id.trim().is_empty() {
        return Err(AstrError::Validation(format!(
            "profile '{}' has a model with empty id",
            profile_name
        )));
    }
    if !seen_model_ids.insert(model.id.clone()) {
        return Err(AstrError::Validation(format!(
            "profile '{}' has duplicate model id '{}'",
            profile_name, model.id
        )));
    }
    if matches!(model.max_tokens, Some(0)) {
        return Err(AstrError::Validation(format!(
            "profile '{}' model '{}' maxTokens must be greater than 0",
            profile_name, model.id
        )));
    }
    if matches!(model.context_limit, Some(0)) {
        return Err(AstrError::Validation(format!(
            "profile '{}' model '{}' contextLimit must be greater than 0",
            profile_name, model.id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_profiles_fails() {
        let mut config = Config::default();
        config.profiles.clear();
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn valid_default_config_passes() {
        let config = Config::default();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn custom_provider_kind_passes_schema_validation() {
        let config = Config {
            active_profile: "corp".to_string(),
            active_model: "corp-model".to_string(),
            profiles: vec![astrcode_core::Profile {
                name: "corp".to_string(),
                provider_kind: "corp-openai".to_string(),
                base_url: "https://api.example.test".to_string(),
                models: vec![ModelConfig::new("corp-model")],
                ..astrcode_core::Profile::default()
            }],
            ..Config::default()
        };

        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn duplicate_profile_name_fails() {
        let mut config = Config::default();
        config.profiles.push(config.profiles[0].clone());
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn responses_mode_with_empty_base_url_fails() {
        let mut config = Config::default();
        let openai = config
            .profiles
            .iter_mut()
            .find(|profile| profile.name == "openai")
            .expect("openai profile should exist");
        openai.base_url.clear();
        openai.api_mode = Some(OpenAiApiMode::Responses);

        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn zero_threshold_percent_fails() {
        let mut config = Config::default();
        config.runtime.compact_threshold_percent = Some(0);
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn zero_max_tool_concurrency_fails() {
        let mut config = Config::default();
        config.runtime.max_tool_concurrency = Some(0);
        let error = validate_config(&config).expect_err("maxToolConcurrency=0 should fail");
        assert!(error.to_string().contains("runtime.maxToolConcurrency"));
    }

    #[test]
    fn zero_agent_max_subrun_depth_fails() {
        let mut config = Config::default();
        config.runtime.agent = Some(astrcode_core::AgentConfig {
            max_subrun_depth: Some(0),
            ..astrcode_core::AgentConfig::default()
        });
        let error = validate_config(&config).expect_err("maxSubrunDepth=0 should fail");
        assert!(error.to_string().contains("runtime.agent.maxSubrunDepth"));
    }

    #[test]
    fn zero_agent_max_spawn_per_turn_fails() {
        let mut config = Config::default();
        config.runtime.agent = Some(astrcode_core::AgentConfig {
            max_spawn_per_turn: Some(0),
            ..astrcode_core::AgentConfig::default()
        });
        let error = validate_config(&config).expect_err("maxSpawnPerTurn=0 should fail");
        assert!(error.to_string().contains("runtime.agent.maxSpawnPerTurn"));
    }

    #[test]
    fn normalize_fills_blank_version() {
        let config = Config {
            version: String::new(),
            ..Config::default()
        };
        let result = normalize_config(config).expect("normalize should succeed");
        assert_eq!(result.version, "1");
    }

    #[test]
    fn negative_api_session_ttl_fails() {
        let mut config = Config::default();
        config.runtime.api_session_ttl_hours = Some(-1);

        let error = validate_config(&config).expect_err("negative ttl should fail");
        assert!(error.to_string().contains("runtime.apiSessionTtlHours"));
    }
}
