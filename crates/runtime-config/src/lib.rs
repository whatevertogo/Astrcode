//! 运行时配置管理 crate。
//!
//! # 职责
//!
//! 本 crate 负责 Astrcode 运行时配置的完整生命周期：
//! - **加载**：从 `~/.astrcode/config.json` 读取用户配置，支持项目级 overlay 覆盖
//! - **保存**：原子写入策略，跨平台兼容（Windows 三步替换 / Unix rename）
//! - **验证**：schema 校验、provider 合法性检查、active_profile/active_model 交叉验证
//! - **解析**：API key 环境变量解析、模型选择回退逻辑、运行时参数解析
//!
//! # 架构定位
//!
//! 在 crate 依赖关系中，`runtime-config` 是独立 crate，仅依赖 `core` 和 `protocol`。
//! 它不依赖 `runtime` 门面，避免循环依赖。配置数据通过显式类型（[`Config`]、[`Profile`]）
//! 跨边界传递。
//!
//! # 配置存储
//!
//! - 用户级配置：`~/.astrcode/config.json`
//! - 项目级 overlay：`<project>/.astrcode/config.json`（仅覆盖 active_profile/active_model/profiles）
//! - 运行时调优参数（如 max_tool_concurrency）仅存在于用户级配置，因为 `RuntimeService`
//!   拥有单一共享的 `AgentLoop`，项目级隔离在当前架构下无法安全实现
//!
//! # API Key 解析策略
//!
//! Profile 中的 `api_key` 支持三种格式：
//! - `literal:<value>`：直接使用字面值，跳过环境变量解析
//! - `env:<NAME>`：强制从环境变量读取，缺失时报错
//! - 裸值（如 `MY_API_KEY`）：若符合环境变量命名规范（大写字母+数字+下划线且含下划线），
//!   尝试从环境变量读取；若环境变量不存在则作为字面值回退（兼容旧版配置）
//!
//! # v1 设计假设
//! - 缺失字段通过 `Default` 填充，不产生警告
//! - 空白的 `version` / `active_profile` / `active_model` 在加载时自动规范化
//! - `active_profile` / `active_model` 会与 `profiles` 列表交叉验证
//! - `provider_kind` 仅支持 `openai-compatible` 和 `anthropic`
//! - `load_config()` 仅在首次初始化时向 stdout 打印一次提示

// Internal modules
mod api_key;
mod connection;
mod constants;
mod editor;
mod env_resolver;
mod loader;
mod saver;
mod selection;
mod types;
mod validation;

// Public re-exports
pub use connection::test_connection;
pub use constants::{
    max_tool_concurrency, resolve_auto_compact_enabled, resolve_compact_keep_recent_turns,
    resolve_compact_threshold_percent, resolve_continuation_min_delta_tokens,
    resolve_default_token_budget, resolve_max_continuations, resolve_max_tool_concurrency,
    resolve_tool_result_max_bytes, ALL_ASTRCODE_ENV_VARS, ANTHROPIC_API_KEY_ENV,
    ASTRCODE_HOME_DIR_ENV, ASTRCODE_MAX_TOOL_CONCURRENCY_ENV, ASTRCODE_PLUGIN_DIRS_ENV,
    ASTRCODE_TEST_HOME_ENV, BUILD_ENV_VARS, CURRENT_CONFIG_VERSION, DEEPSEEK_API_KEY_ENV,
    DEFAULT_AUTO_COMPACT_ENABLED, DEFAULT_COMPACT_KEEP_RECENT_TURNS,
    DEFAULT_COMPACT_THRESHOLD_PERCENT, DEFAULT_CONTINUATION_MIN_DELTA_TOKENS,
    DEFAULT_MAX_CONTINUATIONS, DEFAULT_MAX_TOOL_CONCURRENCY, DEFAULT_TOKEN_BUDGET,
    DEFAULT_TOOL_RESULT_MAX_BYTES, ENV_REFERENCE_PREFIX, HOME_ENV_VARS, LITERAL_VALUE_PREFIX,
    PLUGIN_ENV_VARS, PROVIDER_API_KEY_ENV_VARS, PROVIDER_KIND_ANTHROPIC, PROVIDER_KIND_OPENAI,
    RUNTIME_ENV_VARS, TAURI_ENV_TARGET_TRIPLE_ENV,
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
pub use selection::{
    list_model_options, resolve_active_selection, resolve_current_model, ActiveSelection,
    CurrentModelSelection, ModelOption,
};
pub use types::{Config, ConfigOverlay, Profile, RuntimeConfig, TestResult};
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
        assert!(
            persisted.contains("\"runtime\": {}"),
            "default config should expose the runtime block for future tuning"
        );
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
        assert_eq!(loaded.runtime, Config::default().runtime);
        assert_eq!(loaded.profiles, Config::default().profiles);
    }

    #[test]
    fn resolve_max_tool_concurrency_prefers_explicit_runtime_config() {
        let _guard = TestEnvGuard::new();
        std::env::set_var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV, "3");

        let resolved = resolve_max_tool_concurrency(&RuntimeConfig {
            max_tool_concurrency: Some(7),
            ..RuntimeConfig::default()
        });

        std::env::remove_var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV);
        assert_eq!(resolved, 7);
    }

    #[test]
    fn resolve_max_tool_concurrency_falls_back_to_env_when_runtime_config_is_unset() {
        let _guard = TestEnvGuard::new();
        std::env::set_var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV, "4");

        let resolved = resolve_max_tool_concurrency(&RuntimeConfig::default());

        std::env::remove_var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV);
        assert_eq!(resolved, 4);
    }

    #[test]
    fn resolve_max_tool_concurrency_falls_back_to_default_when_nothing_is_configured() {
        let _guard = TestEnvGuard::new();
        std::env::remove_var(ASTRCODE_MAX_TOOL_CONCURRENCY_ENV);

        let resolved = resolve_max_tool_concurrency(&RuntimeConfig::default());

        assert_eq!(resolved, DEFAULT_MAX_TOOL_CONCURRENCY);
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
            runtime: RuntimeConfig::default(),
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
            runtime: RuntimeConfig::default(),
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
    fn validate_config_rejects_zero_runtime_max_tool_concurrency() {
        let err = validate_config(&Config {
            runtime: RuntimeConfig {
                max_tool_concurrency: Some(0),
                ..RuntimeConfig::default()
            },
            ..Config::default()
        })
        .expect_err("zero runtime maxToolConcurrency should fail");

        assert!(err
            .to_string()
            .contains("runtime.maxToolConcurrency must be greater than 0"));
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
