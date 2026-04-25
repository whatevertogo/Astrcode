//! server-owned profile resolver bridge。
//!
//! server runtime / state / tests 只依赖这里定义的 resolver contract。

use std::{path::Path, sync::Arc};

use astrcode_core::AgentProfile;

use crate::application_error_bridge::ServerRouteError;

pub(crate) trait ServerProfilePort: Send + Sync {
    fn resolve(&self, working_dir: &Path) -> Result<Arc<Vec<AgentProfile>>, ServerRouteError>;
    fn find_profile(
        &self,
        working_dir: &Path,
        profile_id: &str,
    ) -> Result<AgentProfile, ServerRouteError>;
    fn resolve_global(&self) -> Result<Arc<Vec<AgentProfile>>, ServerRouteError>;
    fn invalidate(&self, working_dir: &Path);
    fn invalidate_global(&self);
    fn invalidate_all(&self);
}

#[derive(Clone)]
pub(crate) struct ServerProfileService {
    port: Arc<dyn ServerProfilePort>,
}

impl ServerProfileService {
    pub(crate) fn new(port: Arc<dyn ServerProfilePort>) -> Self {
        Self { port }
    }

    pub(crate) fn resolve(
        &self,
        working_dir: &Path,
    ) -> Result<Arc<Vec<AgentProfile>>, ServerRouteError> {
        self.port.resolve(working_dir)
    }

    pub(crate) fn find_profile(
        &self,
        working_dir: &Path,
        profile_id: &str,
    ) -> Result<AgentProfile, ServerRouteError> {
        self.port.find_profile(working_dir, profile_id)
    }

    pub(crate) fn resolve_global(&self) -> Result<Arc<Vec<AgentProfile>>, ServerRouteError> {
        self.port.resolve_global()
    }

    pub(crate) fn invalidate(&self, working_dir: &Path) {
        self.port.invalidate(working_dir);
    }

    pub(crate) fn invalidate_global(&self) {
        self.port.invalidate_global();
    }

    pub(crate) fn invalidate_all(&self) {
        self.port.invalidate_all();
    }
}
