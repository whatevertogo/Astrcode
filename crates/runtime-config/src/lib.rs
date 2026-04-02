//! Runtime configuration crate.
//!
//! This crate handles loading, saving, and validation of application configuration,
//! including LLM provider profiles and API key resolution.
//!
//! # v1 Assumptions
//! - Missing fields are filled from `Default` without warnings.
//! - Empty `version` / `active_profile` / `active_model` values are normalized during load.
//! - `active_profile` / `active_model` are cross-validated against `profiles`.
//! - `provider_kind` is validated against the supported providers.
//! - `load_config()` is allowed to print once to stdout during first-time initialization.

// Internal modules
mod api_key;
mod connection;
mod constants;
mod editor;
mod env_resolver;
mod loader;
mod saver;
mod types;
mod validation;

// Public re-exports
pub use connection::test_connection;
pub use constants::{
    ALL_ASTRCODE_ENV_VARS, ANTHROPIC_API_KEY_ENV, ASTRCODE_HOME_DIR_ENV, ASTRCODE_PLUGIN_DIRS_ENV,
    ASTRCODE_TEST_HOME_ENV, BUILD_ENV_VARS, CURRENT_CONFIG_VERSION, DEEPSEEK_API_KEY_ENV,
    ENV_REFERENCE_PREFIX, HOME_ENV_VARS, LITERAL_VALUE_PREFIX, PLUGIN_ENV_VARS,
    PROVIDER_API_KEY_ENV_VARS, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI,
    TAURI_ENV_TARGET_TRIPLE_ENV,
};
pub use editor::open_config_in_editor;
pub use env_resolver::{
    env_reference, is_env_var_name, parse_env_value, resolve_env_value, ParsedEnvValue,
};
pub use loader::{
    config_path, load_config, load_config_from_path, load_config_overlay_from_path,
    load_resolved_config, project_overlay_path,
};
pub use saver::{save_config, save_config_to_path};
pub use types::{Config, ConfigOverlay, Profile, TestResult};
pub use validation::validate_config;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::saver::save_config_to_path;
    use crate::validation::validate_config;
    use astrcode_core::home::ASTRCODE_HOME_DIR_ENV;
    use astrcode_core::test_support::TestEnvGuard;

    use super::*;

    fn unique_env_name() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos();
        format!("MY_UNIQUE_TEST_KEY_{}_{}", std::process::id(), nanos)
    }

    #[test]
    fn config_path_has_expected_suffix() {
        let _guard = TestEnvGuard::new();
        let path = config_path().expect("config_path should resolve");
        let rendered = path.to_string_lossy();
        #[cfg(windows)]
        assert!(rendered.ends_with(".astrcode\\config.json"));
        #[cfg(not(windows))]
        assert!(rendered.ends_with(".astrcode/config.json"));
    }

    #[test]
    fn resolve_env_value_prefers_environment_for_legacy_env_like_defaults() {
        let _guard = TestEnvGuard::new();
        let env_name = unique_env_name();
        std::env::set_var(&env_name, "resolved-from-env");

        let resolved = resolve_env_value(&env_name).expect("env-like default should resolve");

        assert_eq!(resolved, "resolved-from-env");
        std::env::remove_var(&env_name);
    }

    #[test]
    fn resolve_env_value_keeps_legacy_env_like_default_when_env_is_missing() {
        let _guard = TestEnvGuard::new();
        let env_name = unique_env_name();
        std::env::remove_var(&env_name);

        let resolved = resolve_env_value(&env_name).expect("missing env should keep raw value");

        assert_eq!(resolved, env_name);
    }

    #[test]
    fn env_reference_formats_explicit_env_prefix() {
        assert_eq!(
            env_reference(DEEPSEEK_API_KEY_ENV),
            format!("{}{}", ENV_REFERENCE_PREFIX, DEEPSEEK_API_KEY_ENV)
        );
    }

    #[test]
    fn first_load_creates_config_file_with_defaults() {
        let _guard = TestEnvGuard::new();
        std::env::remove_var(DEEPSEEK_API_KEY_ENV);

        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join(".astrcode").join("config.json");
        assert!(!path.exists());

        let loaded = load_config_from_path(&path).expect("first load should succeed");
        assert_eq!(loaded, Config::default());
        assert!(path.exists());

        let persisted =
            std::fs::read_to_string(&path).expect("persisted config should be readable");
        let parsed: Config =
            serde_json::from_str(&persisted).expect("persisted config should be valid json");
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn missing_fields_are_filled_by_defaults() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("config.json");
        std::fs::write(&path, "{\"version\":\"1\"}").expect("test config should be written");

        let loaded = load_config_from_path(&path).expect("load should succeed");
        assert_eq!(loaded.version, "1");
        assert_eq!(loaded.active_profile, Config::default().active_profile);
        assert_eq!(loaded.active_model, Config::default().active_model);
        assert_eq!(loaded.profiles, Config::default().profiles);
    }

    #[test]
    fn invalid_json_returns_error_with_full_path() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("broken.json");
        std::fs::write(&path, "{not-valid-json").expect("broken file should be written");

        let err = load_config_from_path(&path).expect_err("invalid json should fail");
        let err_text = err.to_string();
        assert!(err_text.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn load_config_migrates_blank_version_to_current_version() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"version":"","activeProfile":"deepseek","activeModel":"deepseek-chat"}"#,
        )
        .expect("test config should be written");

        let loaded = load_config_from_path(&path).expect("load should succeed");
        assert_eq!(loaded.version, CURRENT_CONFIG_VERSION);
    }

    #[test]
    fn load_config_rejects_active_model_outside_active_profile() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"version":"1","activeProfile":"deepseek","activeModel":"claude-opus-4-5"}"#,
        )
        .expect("test config should be written");

        let err = load_config_from_path(&path).expect_err("invalid active model should fail");
        assert!(err.to_string().contains("active_model"));
    }

    #[test]
    fn resolve_api_key_reads_env_when_value_looks_like_env_var() {
        let _guard = TestEnvGuard::new();
        let env_name = unique_env_name();
        std::env::set_var(&env_name, "resolved-from-env");

        let profile = Profile {
            name: "default".to_string(),
            api_key: Some(env_name.clone()),
            ..Profile::default()
        };
        let resolved = profile.resolve_api_key().expect("env var should resolve");
        assert_eq!(resolved, "resolved-from-env");

        std::env::remove_var(&env_name);
    }

    #[test]
    fn resolve_api_key_errors_when_explicit_env_var_missing() {
        let _guard = TestEnvGuard::new();
        let env_name = unique_env_name();
        std::env::remove_var(&env_name);

        let profile = Profile {
            name: "default".to_string(),
            api_key: Some(format!("env:{env_name}")),
            ..Profile::default()
        };
        let err = profile
            .resolve_api_key()
            .expect_err("missing env var should fail");
        assert!(err.to_string().contains(&env_name));
    }

    #[test]
    fn resolve_api_key_treats_missing_legacy_env_like_value_as_literal() {
        let _guard = TestEnvGuard::new();
        let env_like_value = unique_env_name();
        std::env::remove_var(&env_like_value);

        let profile = Profile {
            name: "default".to_string(),
            api_key: Some(env_like_value.clone()),
            ..Profile::default()
        };
        let resolved = profile
            .resolve_api_key()
            .expect("missing legacy env-like value should be treated as literal");

        assert_eq!(resolved, env_like_value);
    }

    #[test]
    fn resolve_api_key_returns_plaintext_when_not_env_pattern() {
        let profile = Profile {
            name: "default".to_string(),
            api_key: Some("sk-plaintext".to_string()),
            ..Profile::default()
        };
        let resolved = profile
            .resolve_api_key()
            .expect("plaintext api key should pass");
        assert_eq!(resolved, "sk-plaintext");
    }

    #[test]
    fn resolve_api_key_supports_explicit_literal_prefix() {
        let profile = Profile {
            name: "default".to_string(),
            api_key: Some("literal:MY_TEST_KEY".to_string()),
            ..Profile::default()
        };
        let resolved = profile
            .resolve_api_key()
            .expect("literal prefix should bypass env resolution");

        assert_eq!(resolved, "MY_TEST_KEY");
    }

    #[test]
    fn resolve_api_key_errors_for_missing_value() {
        let profile = Profile {
            name: "custom".to_string(),
            api_key: None,
            ..Profile::default()
        };
        let err = profile
            .resolve_api_key()
            .expect_err("missing api key should fail");
        assert!(err.to_string().contains("custom"));
    }

    #[test]
    fn resolve_api_key_errors_for_blank_value() {
        let profile = Profile {
            name: "custom".to_string(),
            api_key: Some("   ".to_string()),
            ..Profile::default()
        };
        let err = profile
            .resolve_api_key()
            .expect_err("blank api key should fail");
        assert!(err.to_string().contains("custom"));
    }

    #[test]
    fn debug_output_redacts_api_keys() {
        let config = Config::default();

        let rendered = format!("{:?}", config);

        assert!(!rendered.contains(DEEPSEEK_API_KEY_ENV));
        assert!(!rendered.contains(ANTHROPIC_API_KEY_ENV));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn save_config_overwrites_existing_file_with_pretty_json() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join(".astrcode").join("config.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parent should exist");
        std::fs::write(&path, "{\"version\":\"old\"}").expect("seed config should be written");

        let config = Config {
            version: CURRENT_CONFIG_VERSION.to_string(),
            active_profile: "custom".to_string(),
            active_model: "gpt-4o-mini".to_string(),
            profiles: vec![Profile {
                name: "custom".to_string(),
                provider_kind: PROVIDER_KIND_OPENAI.to_string(),
                base_url: "https://example.com".to_string(),
                api_key: Some("MY_TEST_KEY".to_string()),
                models: vec!["gpt-4o-mini".to_string()],
                max_tokens: 8096,
            }],
        };

        save_config_to_path(&path, &config).expect("save should succeed");

        let raw = std::fs::read_to_string(&path).expect("saved config should be readable");
        assert!(raw.contains(&format!("\n  \"version\": \"{}\"", CURRENT_CONFIG_VERSION)));
        let parsed: Config = serde_json::from_str(&raw).expect("saved json should parse");
        assert_eq!(parsed, config);
    }

    #[test]
    fn save_config_replaces_target_from_tmp_file() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join(".astrcode").join("config.json");
        let tmp_path = PathBuf::from(format!("{}.tmp", path.to_string_lossy()));
        let config = Config::default();

        save_config_to_path(&path, &config).expect("save should succeed");

        assert!(path.exists(), "final config should exist");
        assert!(!tmp_path.exists(), "tmp file should be renamed away");
    }

    #[tokio::test]
    async fn test_connection_returns_failure_result_when_api_key_cannot_be_resolved() {
        let profile = Profile {
            name: "custom".to_string(),
            base_url: "https://example.com".to_string(),
            api_key: None,
            ..Profile::default()
        };

        let result = test_connection(&profile, "gpt-4o-mini")
            .await
            .expect("test_connection should not return Err on auth setup failure");

        assert!(!result.success);
        assert_eq!(result.provider, "https://example.com");
        assert_eq!(result.model, "gpt-4o-mini");
        assert!(result.error.is_some());
    }

    #[test]
    fn windows_open_command_includes_empty_title_argument() {
        use crate::editor::platform_open_command;

        let path = PathBuf::from(r"C:\Users\Test User\.astrcode\config.json");
        let command = platform_open_command("windows", &path).expect("command should build");

        assert_eq!(command.program, "cmd");
        assert_eq!(
            command.args,
            vec![
                "/c".to_string(),
                "start".to_string(),
                String::new(),
                path.to_string_lossy().to_string(),
            ]
        );
    }

    #[test]
    fn config_path_prefers_isolated_test_home_over_explicit_override() {
        let guard = TestEnvGuard::new();
        let override_home = tempfile::tempdir().expect("tempdir should be created");
        let previous_override = std::env::var_os(ASTRCODE_HOME_DIR_ENV);

        std::env::set_var(ASTRCODE_HOME_DIR_ENV, override_home.path());
        let path = config_path().expect("config_path should resolve");
        let uses_test_home = path.starts_with(guard.home_dir());

        match previous_override {
            Some(value) => std::env::set_var(ASTRCODE_HOME_DIR_ENV, value),
            None => std::env::remove_var(ASTRCODE_HOME_DIR_ENV),
        }

        assert!(
            uses_test_home,
            "config path should stay under the isolated test home"
        );
    }

    #[test]
    fn validate_config_rejects_empty_profile_names() {
        let err = validate_config(&Config {
            profiles: vec![Profile {
                name: "   ".to_string(),
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect_err("empty profile names should fail");

        assert!(err.to_string().contains("profile name cannot be empty"));
    }

    #[test]
    fn validate_config_rejects_duplicate_profile_names() {
        let profile = Profile::default();
        let err = validate_config(&Config {
            active_profile: profile.name.clone(),
            active_model: profile.models[0].clone(),
            profiles: vec![profile.clone(), profile],
            version: CURRENT_CONFIG_VERSION.to_string(),
        })
        .expect_err("duplicate profile names should fail");

        assert!(err.to_string().contains("duplicate profile name"));
    }

    #[test]
    fn validate_config_rejects_profiles_without_models() {
        let err = validate_config(&Config {
            profiles: vec![Profile {
                models: Vec::new(),
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect_err("profiles without models should fail");

        assert!(err.to_string().contains("at least one model"));
    }

    #[test]
    fn validate_config_rejects_zero_max_tokens() {
        let err = validate_config(&Config {
            profiles: vec![Profile {
                max_tokens: 0,
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect_err("zero max_tokens should fail");

        assert!(err
            .to_string()
            .contains("max_tokens must be greater than 0"));
    }

    #[test]
    fn validate_config_rejects_blank_openai_base_url() {
        let err = validate_config(&Config {
            profiles: vec![Profile {
                base_url: "   ".to_string(),
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect_err("blank openai base url should fail");

        assert!(err.to_string().contains("base_url cannot be empty"));
    }

    #[test]
    fn validate_config_rejects_unsupported_provider_kind() {
        let err = validate_config(&Config {
            profiles: vec![Profile {
                provider_kind: "unknown".to_string(),
                ..Profile::default()
            }],
            ..Config::default()
        })
        .expect_err("unsupported provider should fail");

        assert!(err.to_string().contains("unsupported provider_kind"));
    }

    #[test]
    fn load_resolved_config_applies_project_overlay_without_touching_user_defaults() {
        let _guard = TestEnvGuard::new();
        std::env::remove_var(DEEPSEEK_API_KEY_ENV);
        std::env::remove_var(ANTHROPIC_API_KEY_ENV);

        let base = Config {
            active_profile: "deepseek".to_string(),
            active_model: "deepseek-chat".to_string(),
            profiles: Config::default().profiles,
            ..Config::default()
        };
        save_config(&base).expect("base config should save");

        let project_dir = tempfile::tempdir().expect("tempdir should be created");
        let overlay_path =
            project_overlay_path(project_dir.path()).expect("overlay path should resolve");
        std::fs::create_dir_all(overlay_path.parent().expect("overlay parent"))
            .expect("overlay dir should exist");
        std::fs::write(
            &overlay_path,
            serde_json::to_vec_pretty(&ConfigOverlay {
                active_profile: Some("anthropic".to_string()),
                active_model: Some("claude-opus-4-5".to_string()),
                profiles: None,
            })
            .expect("overlay should serialize"),
        )
        .expect("overlay should be written");

        let resolved =
            load_resolved_config(Some(project_dir.path())).expect("resolved config should load");

        assert_eq!(resolved.active_profile, "anthropic");
        assert_eq!(resolved.active_model, "claude-opus-4-5");
        assert_eq!(
            resolved.profiles, base.profiles,
            "unset overlay fields must preserve user-level values"
        );
    }

    #[test]
    fn project_overlay_path_is_stable_for_equivalent_paths() {
        let _guard = TestEnvGuard::new();
        let project_dir = tempfile::tempdir().expect("tempdir should be created");
        let canonical =
            std::fs::canonicalize(project_dir.path()).expect("path should canonicalize");
        let dotted = canonical.join(".");

        let canonical_path =
            project_overlay_path(&canonical).expect("canonical overlay path should resolve");
        let dotted_path =
            project_overlay_path(&dotted).expect("dotted overlay path should resolve");

        assert_eq!(
            canonical_path, dotted_path,
            "equivalent paths must hash into the same private project config bucket"
        );
    }
}
