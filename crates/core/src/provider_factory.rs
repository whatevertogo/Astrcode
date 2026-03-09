use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::config::{load_config, Profile};
use crate::llm::openai::OpenAiProvider;
use crate::llm::LlmProvider;

pub trait ProviderFactory: Send + Sync {
    fn build(&self) -> Result<Arc<dyn LlmProvider>>;
}

pub type DynProviderFactory = Arc<dyn ProviderFactory>;

pub struct ConfigFileProviderFactory;

impl ProviderFactory for ConfigFileProviderFactory {
    fn build(&self) -> Result<Arc<dyn LlmProvider>> {
        let config = load_config()?;
        let profile = select_profile(&config.profiles, &config.active_profile)?;
        let api_key = profile.resolve_api_key()?;
        let model = resolve_model(profile, &config.active_model)?;

        Ok(Arc::new(OpenAiProvider::new(
            profile.base_url.clone(),
            api_key,
            model,
        )))
    }
}

fn select_profile<'a>(profiles: &'a [Profile], active: &str) -> Result<&'a Profile> {
    profiles
        .iter()
        .find(|profile| profile.name == active)
        .or_else(|| profiles.first())
        .ok_or_else(|| anyhow!("no profiles configured"))
}

fn resolve_model(profile: &Profile, active_model: &str) -> Result<String> {
    if profile.models.iter().any(|model| model == active_model) {
        return Ok(active_model.to_string());
    }

    profile
        .models
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("profile '{}' has no models", profile.name))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
    fn config_file_provider_factory_prefers_active_model_when_present() {
        let _guard = TestEnvGuard::new();

        let config = Config {
            active_model: "model-b".to_string(),
            profiles: vec![Profile {
                api_key: Some("sk-test".to_string()),
                models: vec!["model-a".to_string(), "model-b".to_string()],
                ..Profile::default()
            }],
            ..Config::default()
        };
        save_config(&config).expect("config should save");

        let factory = Arc::new(ConfigFileProviderFactory);
        let provider = factory.build();

        assert!(
            provider.is_ok(),
            "factory should build when active model is valid"
        );
    }

    #[test]
    fn config_file_provider_factory_falls_back_to_first_profile_model() {
        let _guard = TestEnvGuard::new();

        let config = Config {
            active_model: "missing-model".to_string(),
            profiles: vec![Profile {
                api_key: Some("sk-test".to_string()),
                models: vec!["model-a".to_string(), "model-b".to_string()],
                ..Profile::default()
            }],
            ..Config::default()
        };
        save_config(&config).expect("config should save");

        let factory = Arc::new(ConfigFileProviderFactory);
        let provider = factory.build();

        assert!(
            provider.is_ok(),
            "factory should fall back to the first model"
        );
    }

    #[test]
    fn config_file_provider_factory_build_errors_when_profile_has_no_models() {
        let _guard = TestEnvGuard::new();

        let config = Config {
            profiles: vec![Profile {
                api_key: Some("sk-test".to_string()),
                models: vec![],
                ..Profile::default()
            }],
            ..Config::default()
        };
        save_config(&config).expect("config should save");

        let factory = Arc::new(ConfigFileProviderFactory);
        let err = match factory.build() {
            Ok(_) => panic!("empty model list should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("has no models"));
    }
}
