use std::{
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};

use crate::{
    AppState, FrontendBuild,
    auth::{AuthSessionManager, BootstrapAuth},
    bootstrap::APP_HOME_OVERRIDE_ENV,
};

pub(crate) fn server_test_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) struct ServerTestEnvGuard {
    _lock: MutexGuard<'static, ()>,
    _temp_home: tempfile::TempDir,
    previous_home_override: Option<std::ffi::OsString>,
}

impl ServerTestEnvGuard {
    pub(crate) fn new() -> Self {
        let lock = match server_test_env_lock().lock() {
            Ok(lock) => lock,
            Err(poisoned) => poisoned.into_inner(),
        };
        let temp_home = tempfile::tempdir().expect("tempdir should be created");
        let previous_home_override = std::env::var_os(APP_HOME_OVERRIDE_ENV);
        std::env::set_var(APP_HOME_OVERRIDE_ENV, temp_home.path());

        Self {
            _lock: lock,
            _temp_home: temp_home,
            previous_home_override,
        }
    }
}

impl Drop for ServerTestEnvGuard {
    fn drop(&mut self) {
        match &self.previous_home_override {
            Some(value) => std::env::set_var(APP_HOME_OVERRIDE_ENV, value),
            None => std::env::remove_var(APP_HOME_OVERRIDE_ENV),
        }
    }
}

pub(crate) async fn test_state(
    frontend_build: Option<FrontendBuild>,
) -> (AppState, ServerTestEnvGuard) {
    let guard = ServerTestEnvGuard::new();
    let runtime = crate::bootstrap::bootstrap_server_runtime()
        .await
        .expect("server runtime should bootstrap in tests");
    let auth_sessions = Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");

    (
        AppState {
            app: runtime.app,
            governance: runtime.governance,
            auth_sessions,
            bootstrap_auth: BootstrapAuth::new(
                "browser-token".to_string(),
                chrono::Utc::now()
                    .checked_add_signed(
                        chrono::Duration::from_std(Duration::from_secs(60))
                            .expect("duration should convert"),
                    )
                    .expect("expiry should be valid")
                    .timestamp_millis(),
            ),
            frontend_build,
        },
        guard,
    )
}
