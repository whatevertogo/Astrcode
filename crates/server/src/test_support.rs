use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use astrcode_core::{CapabilityRouter, PluginRegistry, RuntimeCoordinator, RuntimeHandle};
use astrcode_runtime::RuntimeService;

use crate::auth::{AuthSessionManager, BootstrapAuth};
use crate::bootstrap::APP_HOME_OVERRIDE_ENV;
use crate::capabilities::RuntimeGovernance;
use crate::{AppState, FrontendBuild};

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

pub(crate) fn test_state(frontend_build: Option<FrontendBuild>) -> (AppState, ServerTestEnvGuard) {
    let guard = ServerTestEnvGuard::new();
    let capabilities = CapabilityRouter::builder()
        .build()
        .expect("empty capability router should build");
    let service = Arc::new(
        RuntimeService::from_capabilities(capabilities).expect("runtime service should initialize"),
    );
    let runtime: Arc<dyn RuntimeHandle> = service.clone();
    let coordinator = Arc::new(RuntimeCoordinator::new(
        runtime,
        Arc::new(PluginRegistry::default()),
        Vec::new(),
    ));
    let runtime_governance = Arc::new(RuntimeGovernance::new(
        Arc::clone(&service),
        Arc::clone(&coordinator),
        Vec::new(),
    ));
    let auth_sessions = Arc::new(AuthSessionManager::default());
    auth_sessions.issue_test_token("browser-token");
    (
        AppState {
            service,
            coordinator,
            runtime_governance,
            auth_sessions,
            bootstrap_auth: BootstrapAuth::new(
                "browser-token".to_string(),
                chrono::Utc::now().timestamp_millis() + 60_000,
            ),
            frontend_build,
        },
        guard,
    )
}
