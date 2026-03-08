//! v1 assumptions:
//! - Missing fields are filled from `Default` without warnings.
//! - `active_profile` / `active_model` are not cross-validated against `profiles`.
//! - `provider_kind` is kept as free-form string; v1 does not enforce enum constraints.
//! - `load_config()` is allowed to print once to stdout during first-time initialization.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

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

fn load_config_from_path(path: &Path) -> Result<Config> {
    if !path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("config path has no parent: {}", path.display()))?;
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create config directory for {}",
                parent.display()
            )
        })?;

        let default_cfg = Config::default();
        let json = serde_json::to_string_pretty(&default_cfg)
            .context("failed to serialize default config")?;
        fs::write(path, json)
            .with_context(|| format!("failed to write config file at {}", path.display()))?;

        println!("Config created at {}，请填写 apiKey", path.display());
        return Ok(default_cfg);
    }

    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read config at {}", path.display()))?;
    serde_json::from_str::<Config>(&raw)
        .with_context(|| format!("failed to parse config at {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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
        let _guard = env_lock().lock().expect("env lock should be acquired");
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
        let _guard = env_lock().lock().expect("env lock should be acquired");
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
        let _guard = env_lock().lock().expect("env lock should be acquired");
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
}
