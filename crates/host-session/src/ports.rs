use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use astrcode_core::{
    CapabilitySpec, ChildSessionNode, DeleteProjectResult, HookEventKey, Result, SessionId,
    SessionMeta, SessionTurnAcquireResult, StorageEvent, StoredEvent, TaskSnapshot, TurnId,
    TurnTerminalKind,
};
use astrcode_governance_contract::{ModeId, SystemPromptBlock};
use astrcode_prompt_contract::{
    PromptCacheGlobalStrategy, PromptCacheHints, PromptDeclaration, SystemPromptLayer,
};
use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AgentState, InputQueueProjection};

// ============================================================================
// HookDispatch — host-session 的 hook dispatch port
// ============================================================================

/// host-session 的 hook dispatch port。
///
/// server 通过 adapter 注入 plugin-host dispatch core，
/// 避免 host-session 直接依赖 plugin-host。
#[async_trait]
pub trait HookDispatch: Send + Sync {
    /// 派发 hook 并返回 effects。
    async fn dispatch_hook(
        &self,
        event: HookEventKey,
        payload: HookEventPayload,
    ) -> Result<Vec<HookEffect>>;
}

// ============================================================================
// EventStore
// ============================================================================

/// host-session owner 的事件存储端口。
#[async_trait]
pub trait EventStore: Send + Sync {
    async fn ensure_session(&self, session_id: &SessionId, working_dir: &Path) -> Result<()>;
    async fn append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent>;
    async fn replay(&self, session_id: &SessionId) -> Result<Vec<StoredEvent>>;
    async fn recover_session(&self, session_id: &SessionId) -> Result<RecoveredSessionState> {
        Ok(RecoveredSessionState {
            checkpoint: None,
            tail_events: self.replay(session_id).await?,
        })
    }
    async fn checkpoint_session(
        &self,
        _session_id: &SessionId,
        _checkpoint: &SessionRecoveryCheckpoint,
    ) -> Result<()> {
        Ok(())
    }
    async fn try_acquire_turn(
        &self,
        session_id: &SessionId,
        turn_id: &str,
    ) -> Result<SessionTurnAcquireResult>;
    async fn list_sessions(&self) -> Result<Vec<SessionId>>;
    async fn list_session_metas(&self) -> Result<Vec<SessionMeta>>;
    async fn delete_session(&self, session_id: &SessionId) -> Result<()>;
    async fn delete_sessions_by_working_dir(
        &self,
        working_dir: &str,
    ) -> Result<DeleteProjectResult>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TurnProjectionSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_kind: Option<TurnTerminalKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionRegistrySnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_mode_changed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub child_nodes: HashMap<String, ChildSessionNode>,
    #[serde(default)]
    pub active_tasks: HashMap<String, TaskSnapshot>,
    #[serde(default)]
    pub input_queue_projection_index: HashMap<String, InputQueueProjection>,
    #[serde(default)]
    pub turn_projections: HashMap<String, TurnProjectionSnapshot>,
}

impl ProjectionRegistrySnapshot {
    pub fn is_empty(&self) -> bool {
        self.last_mode_changed_at.is_none()
            && self.child_nodes.is_empty()
            && self.active_tasks.is_empty()
            && self.input_queue_projection_index.is_empty()
            && self.turn_projections.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecoveryCheckpoint {
    pub agent_state: AgentState,
    #[serde(default, skip_serializing_if = "ProjectionRegistrySnapshot::is_empty")]
    pub projection_registry: ProjectionRegistrySnapshot,
    pub checkpoint_storage_seq: u64,
}

impl SessionRecoveryCheckpoint {
    pub fn new(
        agent_state: AgentState,
        projection_registry: ProjectionRegistrySnapshot,
        checkpoint_storage_seq: u64,
    ) -> Self {
        Self {
            agent_state,
            projection_registry,
            checkpoint_storage_seq,
        }
    }

    pub fn projection_registry_snapshot(&self) -> ProjectionRegistrySnapshot {
        self.projection_registry.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredSessionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<SessionRecoveryCheckpoint>,
    #[serde(default)]
    pub tail_events: Vec<StoredEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptGovernanceContext {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_capability_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<ModeId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub approval_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub policy_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_subrun_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_spawn_per_turn: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptFactsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub working_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_capability_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance: Option<PromptGovernanceContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptEntrySummary {
    pub id: String,
    pub description: String,
}

impl PromptEntrySummary {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }
}

pub type PromptSkillSummary = PromptEntrySummary;
pub type PromptAgentProfileSummary = PromptEntrySummary;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptFacts {
    pub profile: String,
    #[serde(default)]
    pub profile_context: Value,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub skills: Vec<PromptSkillSummary>,
    #[serde(default)]
    pub agent_profiles: Vec<PromptAgentProfileSummary>,
    #[serde(default)]
    pub prompt_declarations: Vec<PromptDeclaration>,
}

impl Default for PromptFacts {
    fn default() -> Self {
        Self {
            profile: "coding".to_string(),
            profile_context: Value::Null,
            metadata: Value::Null,
            skills: Vec::new(),
            agent_profiles: Vec::new(),
            prompt_declarations: Vec::new(),
        }
    }
}

#[async_trait]
pub trait PromptFactsProvider: Send + Sync {
    async fn resolve_prompt_facts(&self, request: &PromptFactsRequest) -> Result<PromptFacts>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBuildRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub working_dir: PathBuf,
    pub profile: String,
    #[serde(default)]
    pub step_index: usize,
    #[serde(default)]
    pub turn_index: usize,
    #[serde(default)]
    pub profile_context: Value,
    #[serde(default)]
    pub capabilities: Vec<CapabilitySpec>,
    #[serde(default)]
    pub skills: Vec<PromptSkillSummary>,
    #[serde(default)]
    pub agent_profiles: Vec<PromptAgentProfileSummary>,
    #[serde(default)]
    pub prompt_declarations: Vec<PromptDeclaration>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptBuildCacheMetrics {
    pub reuse_hits: u32,
    pub reuse_misses: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchanged_layers: Vec<SystemPromptLayer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBuildOutput {
    pub system_prompt: String,
    #[serde(default)]
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
    #[serde(default)]
    pub prompt_cache_hints: PromptCacheHints,
    #[serde(default)]
    pub cache_metrics: PromptBuildCacheMetrics,
    #[serde(default)]
    pub metadata: Value,
}

#[async_trait]
pub trait PromptProvider: Send + Sync {
    async fn build_prompt(&self, request: PromptBuildRequest) -> Result<PromptBuildOutput>;
}

impl PromptBuildOutput {
    pub fn empty() -> Self {
        Self {
            system_prompt: String::new(),
            system_prompt_blocks: Vec::new(),
            prompt_cache_hints: PromptCacheHints {
                global_cache_strategy: PromptCacheGlobalStrategy::SystemPrompt,
                ..PromptCacheHints::default()
            },
            cache_metrics: PromptBuildCacheMetrics::default(),
            metadata: Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProjectionRegistrySnapshot, RecoveredSessionState};

    #[test]
    fn projection_registry_empty_checks_all_owned_indexes() {
        let snapshot = ProjectionRegistrySnapshot::default();

        assert!(snapshot.is_empty());
    }

    #[test]
    fn recovered_session_state_defaults_to_tail_replay_only() {
        let recovered = RecoveredSessionState::default();

        assert!(recovered.checkpoint.is_none());
        assert!(recovered.tail_events.is_empty());
    }
}
