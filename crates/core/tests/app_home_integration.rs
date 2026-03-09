use std::sync::{Mutex, OnceLock};

use astrcode_core::{config, EventLog};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn config_path_uses_real_home_by_default_in_non_test_runtime() {
    let _lock = env_lock().lock().expect("env lock should be acquired");
    let previous_override = std::env::var_os("ASTRCODE_HOME_DIR");
    std::env::remove_var("ASTRCODE_HOME_DIR");

    let path = config::config_path().expect("config path should resolve");
    let expected = dirs::home_dir()
        .expect("home directory should resolve")
        .join(".astrcode")
        .join("config.json");

    match previous_override {
        Some(value) => std::env::set_var("ASTRCODE_HOME_DIR", value),
        None => std::env::remove_var("ASTRCODE_HOME_DIR"),
    }

    assert_eq!(path, expected);
}

#[test]
fn event_log_respects_explicit_home_override_in_non_test_runtime() {
    let _lock = env_lock().lock().expect("env lock should be acquired");
    let previous_override = std::env::var_os("ASTRCODE_HOME_DIR");
    let temp = tempfile::tempdir().expect("tempdir should be created");

    std::env::set_var("ASTRCODE_HOME_DIR", temp.path());
    let session_id = "2026-03-09T12-00-00-override";
    let log = EventLog::create(session_id).expect("event log should be created under override");
    let path = log.path().to_path_buf();

    match previous_override {
        Some(value) => std::env::set_var("ASTRCODE_HOME_DIR", value),
        None => std::env::remove_var("ASTRCODE_HOME_DIR"),
    }

    assert!(path.starts_with(temp.path()));
    assert!(path.ends_with(format!("session-{session_id}.jsonl")));
}
