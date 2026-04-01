use std::path::PathBuf;
use std::sync::Arc;

use astrcode_core::{AstrError, Result};

use crate::config::{load_resolved_config, Profile, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI};
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::openai::OpenAiProvider;
use crate::llm::LlmProvider;

pub trait ProviderFactory: Send + Sync {
    fn build_for_working_dir(&self, working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>>;
}

pub type DynProviderFactory = Arc<dyn ProviderFactory>;

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
    fn build_for_working_dir(&self, working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>> {
        let config = load_resolved_config(working_dir.as_deref())?;
        let profile = select_profile(&config.profiles, &config.active_profile)?;
        let model = resolve_model(profile, &config.active_model)?;
        let provider = build_provider(profile, model)?;
        Ok(provider.into_dyn())
    }
}

fn build_provider(profile: &Profile, model: String) -> Result<BuiltProvider> {
    let api_key = profile.resolve_api_key()?;

    match profile.provider_kind.as_str() {
        PROVIDER_KIND_OPENAI => {
            if profile.base_url.trim().is_empty() {
                return Err(AstrError::MissingBaseUrl(format!(
                    "openai-compatible profile '{}' 缺少 baseUrl",
                    profile.name
                )));
            }

            Ok(BuiltProvider::OpenAi(OpenAiProvider::new(
                profile.base_url.clone(),
                api_key,
                model,
                profile.max_tokens,
            )))
        }
        PROVIDER_KIND_ANTHROPIC => Ok(BuiltProvider::Anthropic(
            AnthropicProvider::with_max_tokens(api_key, model, profile.max_tokens),
        )),
        other => Err(AstrError::UnsupportedProvider(other.to_string())),
    }
}

fn select_profile<'a>(profiles: &'a [Profile], active: &str) -> Result<&'a Profile> {
    profiles
        .iter()
        .find(|profile| profile.name == active)
        .or_else(|| profiles.first())
        .ok_or(AstrError::NoProfilesConfigured)
}

fn resolve_model(profile: &Profile, active_model: &str) -> Result<String> {
    if profile.models.iter().any(|model| model == active_model) {
        return Ok(active_model.to_string());
    }

    profile
        .models
        .first()
        .cloned()
        .ok_or_else(|| AstrError::ModelNotFound {
            profile: profile.name.clone(),
            model: String::new(),
        })
}

#[cfg(test)]
mod tests {
    use crate::config::{save_config, Config};
    use crate::test_support::TestEnvGuard;

    use super::*;

    #[test]
    fn resolve_model_prefers_active_model_when_present() {
        let profile = Profile {
            models: vec!["model-a".to_string(), "model-b".to_string()],
            ..Profile::default()
        };

        let model = resolve_model(&profile, "model-b").expect("active model should win");
        assert_eq!(model, "model-b");
    }

    #[test]
    fn resolve_model_falls_back_to_first_profile_model() {
        let profile = Profile {
            models: vec!["model-a".to_string(), "model-b".to_string()],
            ..Profile::default()
        };

        let model = resolve_model(&profile, "missing-model").expect("first model should be used");
        assert_eq!(model, "model-a");
    }

    #[test]
    fn resolve_model_errors_when_profile_has_no_models() {
        let profile = Profile {
            name: "custom".to_string(),
            models: vec![],
            ..Profile::default()
        };

        let err = resolve_model(&profile, "missing-model").expect_err("empty models should fail");
        assert!(err.to_string().contains("custom"));
    }

    #[test]
    fn build_provider_uses_openai_branch() {
        let profile = Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://example.com".to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec!["model-a".to_string()],
            max_tokens: 8096,
        };

        let provider = build_provider(&profile, "model-a".to_string()).expect("build should work");
        assert!(matches!(provider, BuiltProvider::OpenAi(_)));
    }

    #[test]
    fn build_provider_errors_when_openai_base_url_is_missing() {
        let profile = Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "   ".to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec!["model-a".to_string()],
            max_tokens: Profile::default().max_tokens,
        };

        let err = build_provider(&profile, "model-a".to_string())
            .expect_err("missing base url should fail");
        assert!(err.to_string().contains("缺少 baseUrl"));
    }

    #[test]
    fn build_provider_uses_anthropic_branch() {
        let profile = Profile {
            name: "anthropic".to_string(),
            provider_kind: PROVIDER_KIND_ANTHROPIC.to_string(),
            base_url: String::new(),
            api_key: Some("sk-ant".to_string()),
            models: vec!["claude".to_string()],
            max_tokens: Profile::default().max_tokens,
        };

        let provider = build_provider(&profile, "claude".to_string()).expect("build should work");
        assert!(matches!(provider, BuiltProvider::Anthropic(_)));
    }

    #[test]
    fn build_provider_errors_when_kind_is_unknown() {
        let profile = Profile {
            name: "custom".to_string(),
            provider_kind: "unknown".to_string(),
            base_url: "https://example.com".to_string(),
            api_key: Some("sk-test".to_string()),
            models: vec!["model-a".to_string()],
            max_tokens: Profile::default().max_tokens,
        };

        let err =
            build_provider(&profile, "model-a".to_string()).expect_err("unknown kind should fail");
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
                models: vec!["model-a".to_string(), "model-b".to_string()],
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
                models: vec!["model-a".to_string(), "model-b".to_string()],
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
}
