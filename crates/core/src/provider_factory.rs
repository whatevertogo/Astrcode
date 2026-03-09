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
    use crate::tools::fs_common::env_lock_for_tests;

    use super::*;

    struct EnvRestoreGuard {
        previous_home: Option<std::ffi::OsString>,
        previous_userprofile: Option<std::ffi::OsString>,
    }

    impl EnvRestoreGuard {
        fn capture() -> Self {
            Self {
                previous_home: std::env::var_os("HOME"),
                previous_userprofile: std::env::var_os("USERPROFILE"),
            }
        }
    }

    impl Drop for EnvRestoreGuard {
        fn drop(&mut self) {
            match &self.previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.previous_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }

    fn set_test_home(path: &std::path::Path) {
        #[cfg(windows)]
        {
            std::env::set_var("USERPROFILE", path);
            std::env::remove_var("HOME");
        }
        #[cfg(not(windows))]
        {
            std::env::set_var("HOME", path);
            std::env::remove_var("USERPROFILE");
        }
    }

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
        let _guard = env_lock_for_tests()
            .lock()
            .expect("env lock should be acquired");
        let _restore = EnvRestoreGuard::capture();
        let temp = tempfile::tempdir().expect("tempdir should be created");
        set_test_home(temp.path());

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
        let _guard = env_lock_for_tests()
            .lock()
            .expect("env lock should be acquired");
        let _restore = EnvRestoreGuard::capture();
        let temp = tempfile::tempdir().expect("tempdir should be created");
        set_test_home(temp.path());

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
        let _guard = env_lock_for_tests()
            .lock()
            .expect("env lock should be acquired");
        let _restore = EnvRestoreGuard::capture();
        let temp = tempfile::tempdir().expect("tempdir should be created");
        set_test_home(temp.path());

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
