use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock};

use uuid::Uuid;

use astrcode_core::config::{Config, Profile};
use astrcode_core::{load_config, save_config};

use super::*;

fn config_env_lock() -> &'static StdMutex<()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
}

struct AppHomeGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<OsString>,
    home: PathBuf,
}

impl AppHomeGuard {
    fn new() -> Self {
        let lock = config_env_lock().lock().expect("lock should work");
        let previous = std::env::var_os("ASTRCODE_HOME_DIR");
        let home = std::env::temp_dir().join(format!("astrcode-handle-{}", Uuid::new_v4()));
        fs::create_dir_all(&home).expect("temp home should exist");
        std::env::set_var("ASTRCODE_HOME_DIR", &home);

        Self {
            _lock: lock,
            previous,
            home,
        }
    }
}

impl Drop for AppHomeGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("ASTRCODE_HOME_DIR", value),
            None => std::env::remove_var("ASTRCODE_HOME_DIR"),
        }
        let _ = fs::remove_dir_all(&self.home);
    }
}

#[tokio::test]
async fn set_model_writes_config_json() {
    let _guard = AppHomeGuard::new();
    save_config(&Config {
        active_profile: "deepseek".to_string(),
        active_model: "model-a".to_string(),
        profiles: vec![Profile {
            name: "deepseek".to_string(),
            models: vec!["model-a".to_string(), "model-b".to_string()],
            api_key: Some("sk-test".to_string()),
            ..Profile::default()
        }],
        ..Config::default()
    })
    .expect("config should save");

    let handle = AgentHandle::new().expect("handle should build");
    handle
        .set_model("deepseek".to_string(), "model-b".to_string())
        .await
        .expect("set_model should succeed");

    let updated = load_config().expect("config should load");
    assert_eq!(updated.active_profile, "deepseek");
    assert_eq!(updated.active_model, "model-b");
}

#[tokio::test]
async fn set_model_errors_for_missing_profile() {
    let _guard = AppHomeGuard::new();
    save_config(&Config {
        profiles: vec![Profile {
            name: "deepseek".to_string(),
            models: vec!["model-a".to_string()],
            api_key: Some("sk-test".to_string()),
            ..Profile::default()
        }],
        ..Config::default()
    })
    .expect("config should save");

    let handle = AgentHandle::new().expect("handle should build");
    let err = handle
        .set_model("missing".to_string(), "model-a".to_string())
        .await
        .expect_err("missing profile should fail");

    assert!(err.contains("profile 'missing' does not exist"));
}

#[tokio::test]
async fn set_model_errors_for_missing_model() {
    let _guard = AppHomeGuard::new();
    save_config(&Config {
        profiles: vec![Profile {
            name: "deepseek".to_string(),
            models: vec!["model-a".to_string()],
            api_key: Some("sk-test".to_string()),
            ..Profile::default()
        }],
        ..Config::default()
    })
    .expect("config should save");

    let handle = AgentHandle::new().expect("handle should build");
    let err = handle
        .set_model("deepseek".to_string(), "model-b".to_string())
        .await
        .expect_err("missing model should fail");

    assert!(err.contains("model 'model-b' does not exist in profile 'deepseek'"));
}

#[tokio::test]
async fn get_current_model_falls_back_without_writing_config() {
    let _guard = AppHomeGuard::new();
    let config = Config {
        active_profile: "missing".to_string(),
        active_model: "missing-model".to_string(),
        profiles: vec![Profile {
            name: "deepseek".to_string(),
            provider_kind: "openai-compatible".to_string(),
            models: vec!["model-a".to_string(), "model-b".to_string()],
            api_key: Some("sk-test".to_string()),
            ..Profile::default()
        }],
        ..Config::default()
    };
    save_config(&config).expect("config should save");

    let handle = AgentHandle::new().expect("handle should build");
    let current = handle
        .get_current_model()
        .await
        .expect("get_current_model should succeed");

    assert_eq!(current.profile_name, "deepseek");
    assert_eq!(current.model, "model-a");
    assert_eq!(current.provider_kind, "openai-compatible");

    let persisted = load_config().expect("config should still load");
    assert_eq!(persisted.active_profile, "missing");
    assert_eq!(persisted.active_model, "missing-model");
}
