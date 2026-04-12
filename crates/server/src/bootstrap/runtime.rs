//! # 服务器运行时组合根
//!
//! 由 server 显式组装 adapter、kernel、session-runtime、application。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use astrcode_application::{
    App, AppGovernance, ObservabilitySnapshotProvider, RuntimeObservabilitySnapshot,
    SessionInfoProvider, lifecycle::TaskRegistry,
};
use astrcode_core::{
    EventStore, LlmOutput, LlmProvider, ModelLimits, PluginRegistry, PromptBuildOutput,
    PromptBuildRequest, PromptProvider, ResourceProvider, ResourceReadResult,
    ResourceRequestContext, Result, RuntimeCoordinator, RuntimeHandle, SessionId, StorageEvent,
    StoredEvent,
};
use astrcode_kernel::{CapabilityRouter, Kernel, KernelBuilder};
use astrcode_session_runtime::SessionRuntime;
use async_trait::async_trait;

/// 服务器运行时：组合根输出。
pub struct ServerRuntime {
    pub app: Arc<App>,
    pub governance: Arc<AppGovernance>,
}

/// 构建服务器运行时组合根。
pub async fn bootstrap_server_runtime() -> Result<ServerRuntime> {
    let kernel = Arc::new(build_kernel()?);
    let event_store: Arc<dyn EventStore> = Arc::new(InMemoryEventStore::default());
    let session_runtime = Arc::new(SessionRuntime::new(kernel.clone(), event_store));
    let app = Arc::new(App::new(kernel.clone(), session_runtime.clone()));

    let runtime: Arc<dyn RuntimeHandle> = Arc::new(AppRuntimeHandle {});
    let coordinator = Arc::new(RuntimeCoordinator::new(
        runtime,
        Arc::new(PluginRegistry::default()),
        kernel.surface().snapshot().capability_specs,
    ));
    let governance = Arc::new(AppGovernance::new(
        coordinator,
        Arc::new(TaskRegistry::new()),
        Arc::new(DefaultObservability),
        Arc::new(SessionRuntimeInfo {
            session_runtime: session_runtime.clone(),
        }),
    ));

    Ok(ServerRuntime { app, governance })
}

fn build_kernel() -> Result<Kernel> {
    KernelBuilder::default()
        .with_capabilities(CapabilityRouter::default())
        .with_llm_provider(Arc::new(NoopLlmProvider))
        .with_prompt_provider(Arc::new(NoopPromptProvider))
        .with_resource_provider(Arc::new(NoopResourceProvider))
        .build()
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))
}

#[derive(Debug)]
struct AppRuntimeHandle;

#[async_trait]
impl RuntimeHandle for AppRuntimeHandle {
    fn runtime_name(&self) -> &'static str {
        "astrcode-application"
    }

    fn runtime_kind(&self) -> &'static str {
        "application"
    }

    async fn shutdown(
        &self,
        _timeout_secs: u64,
    ) -> std::result::Result<(), astrcode_core::AstrError> {
        Ok(())
    }
}

#[derive(Debug)]
struct DefaultObservability;

impl ObservabilitySnapshotProvider for DefaultObservability {
    fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        RuntimeObservabilitySnapshot::default()
    }
}

struct SessionRuntimeInfo {
    session_runtime: Arc<SessionRuntime>,
}

impl std::fmt::Debug for SessionRuntimeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRuntimeInfo").finish()
    }
}

impl SessionInfoProvider for SessionRuntimeInfo {
    fn loaded_session_count(&self) -> usize {
        self.session_runtime.list_sessions().len()
    }

    fn running_session_ids(&self) -> Vec<String> {
        self.session_runtime
            .list_sessions()
            .into_iter()
            .map(|id| id.to_string())
            .collect()
    }
}

#[derive(Default)]
struct InMemoryEventStore {
    events: Mutex<HashMap<String, Vec<StoredEvent>>>,
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent> {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let list = guard.entry(session_id.to_string()).or_default();
        let stored = StoredEvent {
            storage_seq: (list.len() as u64) + 1,
            event: event.clone(),
        };
        list.push(stored.clone());
        Ok(stored)
    }

    async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>> {
        let guard = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(guard.get(session_id.as_str()).cloned().unwrap_or_default())
    }

    async fn list_sessions(&self) -> Result<Vec<SessionId>> {
        let guard = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(guard.keys().cloned().map(SessionId::from).collect())
    }

    async fn delete_session(&self, session_id: &SessionId) -> Result<()> {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(session_id.as_str());
        Ok(())
    }
}

#[derive(Debug)]
struct NoopLlmProvider;

#[async_trait]
impl LlmProvider for NoopLlmProvider {
    async fn generate(
        &self,
        _request: astrcode_core::LlmRequest,
        _sink: Option<astrcode_core::LlmEventSink>,
    ) -> Result<LlmOutput> {
        Ok(LlmOutput::default())
    }

    fn model_limits(&self) -> ModelLimits {
        ModelLimits {
            context_window: 128_000,
            max_output_tokens: 8_192,
        }
    }
}

#[derive(Debug)]
struct NoopPromptProvider;

#[async_trait]
impl PromptProvider for NoopPromptProvider {
    async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
        Ok(PromptBuildOutput {
            system_prompt: "You are AstrCode.".to_string(),
            system_prompt_blocks: Vec::new(),
            metadata: serde_json::Value::Null,
        })
    }
}

#[derive(Debug)]
struct NoopResourceProvider;

#[async_trait]
impl ResourceProvider for NoopResourceProvider {
    async fn read_resource(
        &self,
        uri: &str,
        _context: &ResourceRequestContext,
    ) -> Result<ResourceReadResult> {
        Ok(ResourceReadResult {
            uri: uri.to_string(),
            content: serde_json::Value::Null,
            metadata: serde_json::Value::Null,
        })
    }
}
