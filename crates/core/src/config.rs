//! v1 assumptions:
//! - Missing fields are filled from `Default` without warnings.
//! - `active_profile` / `active_model` are not cross-validated against `profiles`.
//! - `provider_kind` is kept as free-form string; v1 does not enforce enum constraints.
//! - `load_config()` is allowed to print once to stdout during first-time initialization.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
            version: "1".to_string(),
            active_profile: "default".to_string(),
            active_model: "deepseek-chat".to_string(),
            profiles: vec![Profile::default()],
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            provider_kind: "openai-compatible".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: Some("DEEPSEEK_API_KEY".to_string()),
            models: vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()],
        }
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
            None => bail!("profile '{}' 未配置 apiKey", self.name),
            Some(s) => s.trim().to_string(),
        };

        if val.is_empty() {
            bail!("profile '{}' 的 apiKey 不能为空", self.name);
        }

        let is_env_var_name = val
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && val.contains('_');

        if is_env_var_name {
            return std::env::var(&val).with_context(|| format!("环境变量 {} 未设置", val));
        }

        Ok(val)
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

pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory"))?;
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
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("config path has no parent: {}", path.display()))?;
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create config directory for {}", parent.display())
        })?;

        let default_cfg = Config::default();
        let json = serde_json::to_string_pretty(&default_cfg)
            .context("failed to serialize default config")?;
        fs::write(path, json)
            .with_context(|| format!("failed to write config file at {}", path.display()))?;

        println!("Config created at {}，请填写 apiKey", path.display());
        return Ok(default_cfg);
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config at {}", path.display()))?;
    serde_json::from_str::<Config>(&raw)
        .with_context(|| format!("failed to parse config at {}", path.display()))
}

fn save_config_to_path(path: &Path, config: &Config) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create config directory for {}", parent.display()))?;

    let json = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, json)
        .with_context(|| format!("failed to write temp config at {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to replace config {} with temp file {}",
            path.display(),
            tmp_path.display()
        )
    })?;
    Ok(())
}

pub async fn test_connection(profile: &Profile, model: &str) -> Result<TestResult> {
    let provider = profile.base_url.trim_end_matches('/').to_string();
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

    let endpoint = format!("{}/v1/chat/completions", provider);
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

    let result = match response {
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
    };

    Ok(result)
}

pub fn open_config_in_editor() -> Result<()> {
    let _ = load_config()?;
    let path = config_path()?;
    let open_command = platform_open_command(std::env::consts::OS, &path)?;
    Command::new(&open_command.program)
        .args(&open_command.args)
        .spawn()
        .with_context(|| format!("failed to open config in editor: {}", path.display()))?;
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
        .ok_or_else(|| anyhow!("config path is not valid utf-8: {}", path.display()))?
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
        other => bail!("unsupported platform: {}", other),
    };

    Ok(command)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::tools::fs_common::env_lock_for_tests;

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
        let path = config_path().expect("config_path should resolve");
        let rendered = path.to_string_lossy();
        #[cfg(windows)]
        assert!(rendered.ends_with(".astrcode\\config.json"));
        #[cfg(not(windows))]
        assert!(rendered.ends_with(".astrcode/config.json"));
    }

    #[test]
    fn first_load_creates_config_file_with_defaults() {
        let _guard = env_lock_for_tests()
            .lock()
            .expect("env lock should be acquired");
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
    fn resolve_api_key_reads_env_when_value_looks_like_env_var() {
        let _guard = env_lock_for_tests()
            .lock()
            .expect("env lock should be acquired");
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
    fn resolve_api_key_returns_error_when_env_var_missing() {
        let _guard = env_lock_for_tests()
            .lock()
            .expect("env lock should be acquired");
        let env_name = unique_env_name();
        std::env::remove_var(&env_name);

        let profile = Profile {
            name: "default".to_string(),
            api_key: Some(env_name.clone()),
            ..Profile::default()
        };
        let err = profile
            .resolve_api_key()
            .expect_err("missing env var should fail");
        assert!(err.to_string().contains(&env_name));
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
    fn save_config_overwrites_existing_file_with_pretty_json() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join(".astrcode").join("config.json");
        fs::create_dir_all(path.parent().expect("parent")).expect("parent should exist");
        fs::write(&path, "{\"version\":\"old\"}").expect("seed config should be written");

        let config = Config {
            version: "2".to_string(),
            active_profile: "custom".to_string(),
            active_model: "gpt-4o-mini".to_string(),
            profiles: vec![Profile {
                name: "custom".to_string(),
                provider_kind: "openai-compatible".to_string(),
                base_url: "https://example.com".to_string(),
                api_key: Some("MY_TEST_KEY".to_string()),
                models: vec!["gpt-4o-mini".to_string()],
            }],
        };

        save_config_to_path(&path, &config).expect("save should succeed");

        let raw = fs::read_to_string(&path).expect("saved config should be readable");
        assert!(raw.contains("\n  \"version\": \"2\""));
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
}
