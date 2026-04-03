//! Provider factory abstraction used by the agent loop execution engine.

use std::path::PathBuf;
use std::sync::Arc;

use astrcode_core::Result;
use astrcode_runtime_llm::LlmProvider;

pub trait ProviderFactory: Send + Sync {
    /// Returns true when provider construction performs blocking I/O and should run on a
    /// blocking pool instead of a Tokio worker.
    fn build_requires_blocking_pool(&self) -> bool {
        false
    }

    fn build_for_working_dir(&self, working_dir: Option<PathBuf>) -> Result<Arc<dyn LlmProvider>>;
}

pub type DynProviderFactory = Arc<dyn ProviderFactory>;
