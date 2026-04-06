//! # LLM Provider 工厂
//!
//! 负责根据工作目录解析有效配置，并构建带有统一模型 limits 的 provider 实例。
//!
//! ## 设计
//!
//! - 配置选择与回退继续委托给 `runtime-config`
//! - OpenAI-compatible 模型 limits 只读取本地逐模型配置
//! - Anthropic 优先走 Models API 拉取权威 limits，本地逐模型配置只作为失败兜底

use std::{path::PathBuf, sync::Arc};

use astrcode_core::{AstrError, Result};
use astrcode_runtime_agent_loop::ProviderFactory;
use astrcode_runtime_llm::{LlmProvider, ModelLimits};
use serde::Deserialize;

use crate::{
    config::{
        ANTHROPIC_VERSION, ModelConfig, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI, Profile,
        load_resolved_config, resolve_anthropic_messages_api_url, resolve_anthropic_models_api_url,
        resolve_model_for_profile, resolve_openai_chat_completions_api_url,
    },
    llm::{anthropic::AnthropicProvider, openai::OpenAiProvider},
};

pub struct ConfigFileProviderFactory;

#[derive(Debug)]
enum BuiltProvider {
    OpenAi(OpenAiProvider),
    Anthropic(AnthropicProvider),
}

impl BuiltProvider {
    fn into_dyn(self) -> Arc<dyn LlmProvider> {
        match self {
            BuiltProvider::OpenAi(provider) => Arc::new(provider),
            BuiltProvider::Anthropic(provider) => Arc::new(provider),
        }
    }
}

impl ProviderFactory for ConfigFileProviderFactory {
    fn build_requires_blocking_pool(&self) -> bool {
        true
    }

    fn build_for_working_dir(&self, working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>> {
        let config = load_resolved_config(working_dir.as_deref())?;
        let profile = select_profile(&config.profiles, &config.active_profile)?;
        let model = resolve_model_for_profile(profile, &config.active_model)?.ok_or_else(|| {
            AstrError::ModelNotFound {
                profile: profile.name.clone(),
                model: config.active_model.clone(),
            }
        })?;
        let provider = build_provider(profile, model)?;
        Ok(provider.into_dyn())
    }
}

fn build_provider(profile: &Profile, model: &ModelConfig) -> Result<BuiltProvider> {
    let api_key = profile.resolve_api_key()?;
    let limits = resolve_model_limits(profile, model, &api_key)?;

    match profile.provider_kind.as_str() {
        PROVIDER_KIND_OPENAI => {
            if profile.base_url.trim().is_empty() {
                return Err(AstrError::MissingBaseUrl(format!(
                    "openai-compatible profile '{}' 缺少 baseUrl",
                    profile.name
                )));
            }

            Ok(BuiltProvider::OpenAi(OpenAiProvider::new(
                resolve_openai_chat_completions_api_url(&profile.base_url),
                api_key,
                model.id.clone(),
                limits,
            )?))
        },
        PROVIDER_KIND_ANTHROPIC => Ok(BuiltProvider::Anthropic(AnthropicProvider::new(
            resolve_anthropic_messages_api_url(&profile.base_url),
            api_key,
            model.id.clone(),
            limits,
        )?)),
        other => Err(AstrError::UnsupportedProvider(other.to_string())),
    }
}

fn resolve_model_limits(
    profile: &Profile,
    model: &ModelConfig,
    api_key: &str,
) -> Result<ModelLimits> {
    match profile.provider_kind.as_str() {
        PROVIDER_KIND_OPENAI => resolve_openai_model_limits(profile, model),
        PROVIDER_KIND_ANTHROPIC => resolve_anthropic_model_limits(profile, model, api_key),
        other => Err(AstrError::UnsupportedProvider(other.to_string())),
    }
}

fn resolve_openai_model_limits(profile: &Profile, model: &ModelConfig) -> Result<ModelLimits> {
    model_limits_from_local_override(model).ok_or_else(|| {
        AstrError::Validation(format!(
            "openai-compatible profile '{}' model '{}' must set both maxTokens and contextLimit",
            profile.name, model.id
        ))
    })
}

fn resolve_anthropic_model_limits(
    profile: &Profile,
    model: &ModelConfig,
    api_key: &str,
) -> Result<ModelLimits> {
    let local_limits = model_limits_from_local_override(model);
    let models_api_url = resolve_anthropic_models_api_url(&profile.base_url);

    match fetch_anthropic_model_metadata(&models_api_url, api_key, &model.id) {
        Ok(metadata) => metadata
            .into_model_limits(&model.id)
            .or_else(|error| local_limits.ok_or(error)),
        Err(error) => local_limits.ok_or(error),
    }
}

fn model_limits_from_local_override(model: &ModelConfig) -> Option<ModelLimits> {
    match (model.context_limit, model.max_tokens) {
        (Some(context_window), Some(max_output_tokens))
            if context_window > 0 && max_output_tokens > 0 =>
        {
            Some(ModelLimits {
                context_window,
                max_output_tokens: max_output_tokens as usize,
            })
        },
        _ => None,
    }
}

fn fetch_anthropic_model_metadata(
    models_api_url: &str,
    api_key: &str,
    model_id: &str,
) -> Result<AnthropicModelMetadata> {
    let metadata_url = build_model_metadata_url(models_api_url, model_id)?.to_string();
    let api_key = api_key.to_string();

    // provider 构造是同步接口。这里显式起一个独立线程来跑一次短生命周期的 Tokio runtime，
    // 避免在已有 async runtime 上错误地嵌套 `block_on`。
    std::thread::spawn(move || -> Result<AnthropicModelMetadata> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                AstrError::Internal(format!("failed to build metadata runtime: {error}"))
            })?;

        runtime.block_on(async move {
            let response = reqwest::Client::new()
                .get(metadata_url)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .map_err(|error| {
                    AstrError::http("failed to fetch anthropic model metadata", error)
                })?;

            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(AstrError::InvalidApiKey("Anthropic".to_string()));
            }
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(AstrError::LlmRequestFailed {
                    status: status.as_u16(),
                    body,
                });
            }

            response
                .json::<AnthropicModelMetadata>()
                .await
                .map_err(|error| {
                    AstrError::http("failed to parse anthropic model metadata response", error)
                })
        })
    })
    .join()
    .map_err(|_| AstrError::Internal("anthropic metadata thread panicked".to_string()))?
}

fn build_model_metadata_url(models_api_url: &str, model_id: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(models_api_url).map_err(|error| {
        AstrError::Validation(format!(
            "invalid anthropic models api url '{}': {error}",
            models_api_url
        ))
    })?;

    // 使用 URL path_segments 追加模型 id，避免查询参数和尾斜杠把路径拼坏。
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            AstrError::Validation(format!(
                "anthropic models api url '{}' cannot accept path segments",
                models_api_url
            ))
        })?;
        segments.pop_if_empty();
        segments.push(model_id);
    }

    Ok(url)
}

#[derive(Debug, Deserialize)]
struct AnthropicModelMetadata {
    #[serde(default)]
    max_input_tokens: usize,
    #[serde(default)]
    max_tokens: usize,
}

impl AnthropicModelMetadata {
    fn into_model_limits(self, model_id: &str) -> Result<ModelLimits> {
        if self.max_input_tokens == 0 || self.max_tokens == 0 {
            return Err(AstrError::Validation(format!(
                "anthropic model '{}' returned invalid limits: max_input_tokens={}, max_tokens={}",
                model_id, self.max_input_tokens, self.max_tokens
            )));
        }

        Ok(ModelLimits {
            context_window: self.max_input_tokens,
            max_output_tokens: self.max_tokens,
        })
    }
}

/// 选择活跃配置。若 active 名称不匹配任何 profile，静默回退到第一个。
/// 这是一种宽容降级策略：配置验证通常能阻止不匹配的配置写入，但运行时容错允许
/// 手工编辑配置后仍能以第一个可用 profile 启动。
fn select_profile<'a>(profiles: &'a [Profile], active: &str) -> Result<&'a Profile> {
    profiles
        .iter()
        .find(|profile| profile.name == active)
        .or_else(|| profiles.first())
        .ok_or(AstrError::NoProfilesConfigured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, save_config},
        test_support::TestEnvGuard,
    };

    fn model(id: &str) -> ModelConfig {
        ModelConfig::new(id)
    }

    #[test]
    fn resolve_openai_model_limits_requires_manual_values() {
        let profile = Profile {
            name: "openai".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            models: vec![model("gpt-4o")],
            ..Profile::default()
        };

        let error = resolve_openai_model_limits(&profile, &profile.models[0])
            .expect_err("missing manual limits should fail");

        assert!(
            error
                .to_string()
                .contains("must set both maxTokens and contextLimit")
        );
    }

    #[test]
    fn resolve_anthropic_model_limits_uses_local_fallback_when_remote_fails() {
        let model = ModelConfig {
            id: "claude".to_string(),
            max_tokens: Some(4096),
            context_limit: Some(200_000),
        };

        let profile = Profile {
            name: "anthropic".to_string(),
            provider_kind: PROVIDER_KIND_ANTHROPIC.to_string(),
            base_url: "https://gateway.example.com/anthropic".to_string(),
            api_key: Some("sk-ant".to_string()),
            models: vec![model.clone()],
        };

        let limits = resolve_anthropic_model_limits(&profile, &model, "sk-test")
            .expect("local fallback should be accepted when remote call fails");

        assert_eq!(
            limits,
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 4096,
            }
        );
    }

    #[test]
    fn build_provider_uses_openai_branch() {
        let profile = Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://example.com".to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec![ModelConfig {
                id: "model-a".to_string(),
                max_tokens: Some(8096),
                context_limit: Some(128_000),
            }],
        };

        let provider = build_provider(&profile, &profile.models[0]).expect("build should work");
        assert!(matches!(provider, BuiltProvider::OpenAi(_)));
    }

    #[test]
    fn build_provider_errors_when_openai_base_url_is_missing() {
        let profile = Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "   ".to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec![ModelConfig {
                id: "model-a".to_string(),
                max_tokens: Some(8096),
                context_limit: Some(128_000),
            }],
        };

        let err =
            build_provider(&profile, &profile.models[0]).expect_err("missing base url should fail");
        assert!(err.to_string().contains("缺少 baseUrl"));
    }

    #[test]
    fn build_provider_uses_anthropic_branch_when_local_limits_exist() {
        let profile = Profile {
            name: "anthropic".to_string(),
            provider_kind: PROVIDER_KIND_ANTHROPIC.to_string(),
            base_url: String::new(),
            api_key: Some("sk-ant".to_string()),
            models: vec![ModelConfig {
                id: "claude".to_string(),
                max_tokens: Some(4096),
                context_limit: Some(200_000),
            }],
        };

        let provider = build_provider(&profile, &profile.models[0]).expect("build should work");
        assert!(matches!(provider, BuiltProvider::Anthropic(_)));
    }

    #[test]
    fn build_provider_errors_when_kind_is_unknown() {
        let profile = Profile {
            name: "custom".to_string(),
            provider_kind: "unknown".to_string(),
            base_url: "https://example.com".to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec![ModelConfig {
                id: "model-a".to_string(),
                max_tokens: Some(8096),
                context_limit: Some(128_000),
            }],
        };

        let err =
            build_provider(&profile, &profile.models[0]).expect_err("unknown kind should fail");
        assert!(err.to_string().contains("unsupported provider"));
    }

    #[test]
    fn config_file_provider_factory_prefers_active_model_when_present() {
        let _guard = TestEnvGuard::new();

        let config = Config {
            active_profile: "deepseek".to_string(),
            active_model: "model-b".to_string(),
            profiles: vec![Profile {
                api_key: Some("sk-test".to_string()),
                models: vec![
                    ModelConfig {
                        id: "model-a".to_string(),
                        max_tokens: Some(8096),
                        context_limit: Some(128_000),
                    },
                    ModelConfig {
                        id: "model-b".to_string(),
                        max_tokens: Some(8096),
                        context_limit: Some(128_000),
                    },
                ],
                ..Profile::default()
            }],
            ..Config::default()
        };
        save_config(&config).expect("config should save");

        let factory = ConfigFileProviderFactory;
        let provider = factory.build_for_working_dir(None);

        assert!(
            provider.is_ok(),
            "factory should build when active model is valid"
        );
    }

    #[test]
    fn save_config_rejects_active_model_missing_from_active_profile() {
        let _guard = TestEnvGuard::new();

        let config = Config {
            active_profile: "deepseek".to_string(),
            active_model: "missing-model".to_string(),
            profiles: vec![Profile {
                api_key: Some("sk-test".to_string()),
                models: vec![
                    ModelConfig {
                        id: "model-a".to_string(),
                        max_tokens: Some(8096),
                        context_limit: Some(128_000),
                    },
                    ModelConfig {
                        id: "model-b".to_string(),
                        max_tokens: Some(8096),
                        context_limit: Some(128_000),
                    },
                ],
                ..Profile::default()
            }],
            ..Config::default()
        };
        let err = save_config(&config).expect_err("invalid active model should be rejected");
        assert!(err.to_string().contains("active_model"));
    }

    #[test]
    fn save_config_rejects_profile_without_models() {
        let _guard = TestEnvGuard::new();

        let config = Config {
            profiles: vec![Profile {
                api_key: Some("sk-test".to_string()),
                models: vec![],
                ..Profile::default()
            }],
            ..Config::default()
        };
        let err = save_config(&config).expect_err("empty model list should fail");
        assert!(err.to_string().contains("at least one model"));
    }

    #[test]
    fn build_model_metadata_url_preserves_query_parameters() {
        let url = build_model_metadata_url(
            "https://gateway.example.com/anthropic/v1/models?foo=bar",
            "claude-sonnet-4-5",
        )
        .expect("metadata url should build");

        assert_eq!(
            url.as_str(),
            "https://gateway.example.com/anthropic/v1/models/claude-sonnet-4-5?foo=bar"
        );
    }
}
