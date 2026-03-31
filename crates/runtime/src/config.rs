//! v1 assumptions:
//! - Missing fields are filled from `Default` without warnings.
//! - Empty `version` / `active_profile` / `active_model` values are normalized during load.
//! - `active_profile` / `active_model` are cross-validated against `profiles`.
//! - `provider_kind` is validated against the supported providers.
//! - `load_config()` is allowed to print once to stdout during first-time initialization.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use astrcode_core::{AstrError, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const PROVIDER_KIND_OPENAI: &str = "openai-compatible";
pub const PROVIDER_KIND_ANTHROPIC: &str = "anthropic";
pub const ANTHROPIC_MESSAGES_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const CURRENT_CONFIG_VERSION: &str = "1";

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Config {
    #[serde(default = "default_config_version")]
    pub version: String,
    #[serde(default = "default_config_active_profile")]
    pub active_profile: String,
    #[serde(default = "default_config_active_model")]
    pub active_model: String,
    #[serde(default = "default_config_profiles")]
    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION.to_string(),
            active_profile: "deepseek".to_string(),
            active_model: "deepseek-chat".to_string(),
            profiles: default_profiles(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Profile {
    #[serde(default = "default_profile_name")]
    pub name: String,
    #[serde(default = "default_profile_provider_kind")]
    pub provider_kind: String,
    #[serde(default = "default_profile_base_url")]
    pub base_url: String,
    #[serde(default = "default_profile_api_key")]
    pub api_key: Option<String>,
    #[serde(default = "default_profile_models")]
    pub models: Vec<String>,
    #[serde(default = "default_profile_max_tokens")]
    pub max_tokens: u32,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some("env:DEEPSEEK_API_KEY".to_string()),
            models: vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()],
            max_tokens: 8096,
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("version", &self.version)
            .field("active_profile", &self.active_profile)
            .field("active_model", &self.active_model)
            .field("profiles", &self.profiles)
            .finish()
    }
}

impl fmt::Debug for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Profile")
            .field("name", &self.name)
            .field("provider_kind", &self.provider_kind)
            .field("base_url", &self.base_url)
            .field("api_key", &redacted_api_key(self.api_key.as_deref()))
            .field("models", &self.models)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TestResult {
    pub success: bool,
    pub provider: String,
    pub model: String,
    pub error: Option<String>,
}

impl Profile {
    pub fn resolve_api_key(&self) -> Result<String> {
        let val = match &self.api_key {
            None => {
                return Err(AstrError::MissingApiKey(format!(
                    "profile '{}' 未配置 apiKey",
                    self.name
                )))
            }
            Some(s) => s.trim().to_string(),
        };

        if val.is_empty() {
            return Err(AstrError::MissingApiKey(format!(
                "profile '{}' 的 apiKey 不能为空",
                self.name
            )));
        }

        if let Some(raw) = val.strip_prefix("literal:") {
            let literal = raw.trim().to_string();
            if literal.is_empty() {
                return Err(AstrError::MissingApiKey(format!(
                    "profile '{}' 的 apiKey 不能为空",
                    self.name
                )));
            }
            return Ok(literal);
        }

        if let Some(raw) = val.strip_prefix("env:") {
            let env_name = raw.trim();
            if !is_env_var_name(env_name) {
                return Err(AstrError::Validation(format!(
                    "profile '{}' 的 apiKey env 引用 '{}' 非法",
                    self.name, env_name
                )));
            }
            return std::env::var(env_name)
                .map_err(|_| AstrError::EnvVarNotFound(format!("环境变量 {} 未设置", env_name)));
        }

        if is_env_var_name(&val) {
            if let Ok(resolved) = std::env::var(&val) {
                return Ok(resolved);
            }
        }

        Ok(val)
    }
}

fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}

fn redacted_api_key(value: Option<&str>) -> &'static str {
    if value.is_some() {
        "<redacted>"
    } else {
        "<unset>"
    }
}

fn default_config_version() -> String {
    Config::default().version
}

fn default_config_active_profile() -> String {
    Config::default().active_profile
}

fn default_config_active_model() -> String {
    Config::default().active_model
}

fn default_config_profiles() -> Vec<Profile> {
    Config::default().profiles
}

fn default_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "deepseek".to_string(),
            provider_kind: PROVIDER_KIND_OPENAI.to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some("env:DEEPSEEK_API_KEY".to_string()),
            models: vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()],
            max_tokens: 8096,
        },
        Profile {
            name: "anthropic".to_string(),
            provider_kind: PROVIDER_KIND_ANTHROPIC.to_string(),
            base_url: String::new(),
            api_key: Some("env:ANTHROPIC_API_KEY".to_string()),
            models: vec![
                "claude-sonnet-4-5-20251001".to_string(),
                "claude-opus-4-5".to_string(),
            ],
            max_tokens: 8096,
        },
    ]
}

fn default_profile_name() -> String {
    Profile::default().name
}

fn default_profile_provider_kind() -> String {
    Profile::default().provider_kind
}

fn default_profile_base_url() -> String {
    Profile::default().base_url
}

fn default_profile_api_key() -> Option<String> {
    Profile::default().api_key
}

fn default_profile_models() -> Vec<String> {
    Profile::default().models
}

fn default_profile_max_tokens() -> u32 {
    Profile::default().max_tokens
}

pub fn config_path() -> Result<PathBuf> {
    let home = astrcode_core::home::resolve_home_dir()?;
    Ok(home.join(".astrcode").join("config.json"))
}

pub fn load_config() -> Result<Config> {
    let path = config_path()?;
    load_config_from_path(&path)
}

pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    save_config_to_path(&path, config)
}

fn load_config_from_path(path: &Path) -> Result<Config> {
    if !path.exists() {
        let parent = path.parent().ok_or_else(|| {
            AstrError::Internal(format!("config path has no parent: {}", path.display()))
        })?;
        fs::create_dir_all(parent).map_err(|e| {
            AstrError::io(
                format!("failed to create config directory for {}", parent.display()),
                e,
            )
        })?;

        let default_cfg = Config::default();
        write_json_atomic(path, &default_cfg).map_err(|e| {
            e.context(format!(
                "failed to initialize config file at {}",
                path.display()
            ))
        })?;

        println!("Config created at {}，请填写 apiKey", path.display());
        return Ok(normalize_config(default_cfg)?);
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| AstrError::io(format!("failed to read config at {}", path.display()), e))?;
    let config = serde_json::from_str::<Config>(&raw).map_err(|e| {
        AstrError::parse(format!("failed to parse config at {}", path.display()), e)
    })?;
    normalize_config(config)
        .map_err(|e| e.context(format!("failed to validate config at {}", path.display())))
}

fn save_config_to_path(path: &Path, config: &Config) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        AstrError::Internal(format!("config path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| {
        AstrError::io(
            format!("failed to create config directory for {}", parent.display()),
            e,
        )
    })?;

    let normalized = normalize_config(config.clone())?;
    write_json_atomic(path, &normalized)
}

fn normalize_config(mut config: Config) -> Result<Config> {
    migrate_config(&mut config)?;
    validate_config(&config)?;
    Ok(config)
}

fn migrate_config(config: &mut Config) -> Result<()> {
    if config.version.trim().is_empty() {
        config.version = CURRENT_CONFIG_VERSION.to_string();
    }

    match config.version.as_str() {
        CURRENT_CONFIG_VERSION => {}
        other => {
            return Err(AstrError::Validation(format!(
                "unsupported config version: {}",
                other
            )))
        }
    }

    if config.active_profile.trim().is_empty() {
        config.active_profile = Config::default().active_profile;
    }

    if config.active_model.trim().is_empty() {
        config.active_model = Config::default().active_model;
    }

    Ok(())
}

fn validate_config(config: &Config) -> Result<()> {
    if config.profiles.is_empty() {
        return Err(AstrError::Validation(
            "config must contain at least one profile".to_string(),
        ));
    }

    let mut seen_names = std::collections::HashSet::new();
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

fn write_json_atomic(path: &Path, config: &Config) -> Result<()> {
    let json = serde_json::to_vec_pretty(config)
        .map_err(|e| AstrError::parse("failed to serialize config", e))?;
    let tmp_path = path.with_extension("json.tmp");
    let mut tmp_file = fs::File::create(&tmp_path).map_err(|e| {
        AstrError::io(
            format!("failed to create temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    tmp_file.write_all(&json).map_err(|e| {
        AstrError::io(
            format!("failed to write temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    tmp_file.flush().map_err(|e| {
        AstrError::io(
            format!("failed to flush temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    tmp_file.sync_all().map_err(|e| {
        AstrError::io(
            format!("failed to fsync temp config at {}", tmp_path.display()),
            e,
        )
    })?;
    drop(tmp_file);

    // On most platforms, `rename` will atomically replace the destination.
    // On Windows, `std::fs::rename` fails with `AlreadyExists` if the
    // destination exists, so swap via a backup path and try to roll back
    // if the second rename fails.
    if let Err(err) = fs::rename(&tmp_path, path) {
        #[cfg(windows)]
        {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                let backup_path = path.with_extension("json.bak");
                let _ = fs::remove_file(&backup_path);

                // Move old config out of the way before placing the new file.
                if let Err(backup_err) = fs::rename(path, &backup_path) {
                    let _ = fs::remove_file(&tmp_path);
                    return Err(AstrError::Internal(format!(
                        "failed to move existing config {} to backup {} before replace: {}",
                        path.display(),
                        backup_path.display(),
                        backup_err
                    )));
                }

                if let Err(rename_err) = fs::rename(&tmp_path, path) {
                    match fs::rename(&backup_path, path) {
                        Ok(()) => return Err(AstrError::Internal(format!(
                            "failed to replace config {} with temp file {}: {}; original config restored from backup {} (temp file kept for recovery)",
                            path.display(),
                            tmp_path.display(),
                            rename_err,
                            backup_path.display()
                        ))),
                        Err(restore_err) => return Err(AstrError::Internal(format!(
                            "failed to replace config {} with temp file {}: {}; also failed to restore backup {}: {} (temp file kept for recovery)",
                            path.display(),
                            tmp_path.display(),
                            rename_err,
                            backup_path.display(),
                            restore_err
                        ))),
                    }
                }

                let _ = fs::remove_file(&backup_path);
            } else {
                let _ = fs::remove_file(&tmp_path);
                return Err(AstrError::Internal(format!(
                    "failed to replace config {} with temp file {}: {}",
                    path.display(),
                    tmp_path.display(),
                    err
                )));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(&tmp_path);
            return Err(AstrError::Internal(format!(
                "failed to replace config {} with temp file {}: {}",
                path.display(),
                tmp_path.display(),
                err
            )));
        }
    }
    Ok(())
}

pub async fn test_connection(profile: &Profile, model: &str) -> Result<TestResult> {
    let provider = match profile.provider_kind.as_str() {
        PROVIDER_KIND_ANTHROPIC => ANTHROPIC_MESSAGES_API_URL.to_string(),
        _ => profile.base_url.trim_end_matches('/').to_string(),
    };
    let api_key = match profile.resolve_api_key() {
        Ok(api_key) => api_key,
        Err(err) => {
            return Ok(TestResult {
                success: false,
                provider,
                model: model.to_string(),
                error: Some(err.to_string()),
            });
        }
    };

    match profile.provider_kind.as_str() {
        PROVIDER_KIND_OPENAI => {
            let endpoint = format!("{}/chat/completions", provider);
            let response = reqwest::Client::new()
                .post(endpoint)
                .bearer_auth(api_key)
                .timeout(Duration::from_secs(10))
                .json(&json!({
                    "model": model,
                    "messages": [
                        {
                            "role": "user",
                            "content": "hi"
                        }
                    ],
                    "max_tokens": 1,
                    "stream": false
                }))
                .send()
                .await;

            Ok(connection_result_from_response(response, provider, model))
        }
        PROVIDER_KIND_ANTHROPIC => {
            let response = reqwest::Client::new()
                .post(&provider)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .timeout(Duration::from_secs(10))
                .json(&json!({
                    "model": model,
                    "max_tokens": 1,
                    "messages": [
                        {
                            "role": "user",
                            "content": "hi"
                        }
                    ]
                }))
                .send()
                .await;

            Ok(connection_result_from_response(response, provider, model))
        }
        other => Err(AstrError::UnsupportedProvider(other.to_string())),
    }
}

fn connection_result_from_response(
    response: std::result::Result<reqwest::Response, reqwest::Error>,
    provider: String,
    model: &str,
) -> TestResult {
    match response {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                TestResult {
                    success: true,
                    provider,
                    model: model.to_string(),
                    error: None,
                }
            } else if status == reqwest::StatusCode::UNAUTHORIZED {
                TestResult {
                    success: false,
                    provider,
                    model: model.to_string(),
                    error: Some("API Key 无效或未授权".to_string()),
                }
            } else {
                TestResult {
                    success: false,
                    provider,
                    model: model.to_string(),
                    error: Some(format!("请求失败: {}", status)),
                }
            }
        }
        Err(err) if err.is_timeout() => TestResult {
            success: false,
            provider,
            model: model.to_string(),
            error: Some("连接超时".to_string()),
        },
        Err(err) => TestResult {
            success: false,
            provider,
            model: model.to_string(),
            error: Some(err.to_string()),
        },
    }
}

pub fn open_config_in_editor() -> Result<()> {
    let _ = load_config()?;
    let path = config_path()?;
    let open_command = platform_open_command(std::env::consts::OS, &path)?;
    Command::new(&open_command.program)
        .args(&open_command.args)
        .spawn()
        .map_err(|e| {
            AstrError::io(
                format!("failed to open config in editor: {}", path.display()),
                e,
            )
        })?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenCommand {
    program: String,
    args: Vec<String>,
}

fn platform_open_command(os: &str, path: &Path) -> Result<OpenCommand> {
    let rendered_path = path
        .to_str()
        .ok_or_else(|| {
            AstrError::Internal(format!(
                "config path is not valid utf-8: {}",
                path.display()
            ))
        })?
        .to_string();

    let command = match os {
        "windows" => OpenCommand {
            program: "cmd".to_string(),
            args: vec![
                "/c".to_string(),
                "start".to_string(),
                String::new(),
                rendered_path,
            ],
        },
        "macos" => OpenCommand {
            program: "open".to_string(),
            args: vec![rendered_path],
        },
        "linux" => OpenCommand {
            program: "xdg-open".to_string(),
            args: vec![rendered_path],
        },
        other => return Err(AstrError::UnsupportedPlatform(other.to_string())),
    };

    Ok(command)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::test_support::TestEnvGuard;

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
    fn first_load_creates_config_file_with_defaults() {
        let _guard = TestEnvGuard::new();
        std::env::remove_var("DEEPSEEK_API_KEY");

        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join(".astrcode").join("config.json");
        assert!(!path.exists());

        let loaded = load_config_from_path(&path).expect("first load should succeed");
        assert_eq!(loaded, Config::default());
        assert!(path.exists());

        let persisted = fs::read_to_string(&path).expect("persisted config should be readable");
        let parsed: Config =
            serde_json::from_str(&persisted).expect("persisted config should be valid json");
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn missing_fields_are_filled_by_defaults() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("config.json");
        fs::write(&path, "{\"version\":\"1\"}").expect("test config should be written");

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
        fs::write(&path, "{not-valid-json").expect("broken file should be written");

        let err = load_config_from_path(&path).expect_err("invalid json should fail");
        let err_text = err.to_string();
        assert!(err_text.contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn load_config_migrates_blank_version_to_current_version() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("config.json");
        fs::write(
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
        fs::write(
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

        assert!(!rendered.contains("DEEPSEEK_API_KEY"));
        assert!(!rendered.contains("ANTHROPIC_API_KEY"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn save_config_overwrites_existing_file_with_pretty_json() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join(".astrcode").join("config.json");
        fs::create_dir_all(path.parent().expect("parent")).expect("parent should exist");
        fs::write(&path, "{\"version\":\"old\"}").expect("seed config should be written");

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

        let raw = fs::read_to_string(&path).expect("saved config should be readable");
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
        use astrcode_core::home::ASTRCODE_HOME_DIR_ENV;

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
}
