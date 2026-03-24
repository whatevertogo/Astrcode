use std::sync::Arc;

use crate::{Orchestrator, PluginRegistry};

pub struct RuntimeCoordinator {
    pub active_runtime: Arc<dyn Orchestrator>,
    pub registry: Arc<PluginRegistry>,
}
