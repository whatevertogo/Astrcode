//! # Astrcode 应用层
//!
//! 纯业务编排层，不依赖任何 adapter-* crate，只依赖 core / kernel / session-runtime。
//!
//! 核心职责：
//! - 通过 `App` 结构体暴露所有业务用例入口
//! - 持有并编排 governance surface（治理面）、mode catalog（模式目录）等基础设施
//! - 通过 port trait 与 adapter 层解耦（AppKernelPort / AppSessionPort / ComposerSkillPort）

use std::{path::Path, sync::Arc};

use astrcode_core::AgentProfile;
use tokio::sync::broadcast;

use crate::config::ConfigService;

mod agent_use_cases;
mod governance_surface;
mod ports;
mod session_plan;
mod session_use_cases;
mod terminal_queries;
#[cfg(test)]
mod test_support;

pub mod agent;
pub mod composer;
pub mod config;
pub mod errors;
pub mod execution;
pub mod lifecycle;
pub mod mcp;
pub mod mode;
pub mod observability;
pub mod terminal;
pub mod watch;

pub use agent::AgentOrchestrationService;
pub use astrcode_core::{
    AgentEvent, AgentEventContext, AgentLifecycleStatus, AgentMode, AgentTurnOutcome, ArtifactRef,
    AstrError, CapabilitySpec, ChildAgentRef, ChildSessionLineageKind,
    ChildSessionNotificationKind, CompactTrigger, ComposerOption, ComposerOptionActionKind,
    ComposerOptionKind, Config, ExecutionAccepted, ForkMode, InvocationKind, InvocationMode,
    LocalServerInfo, Phase, PluginHealth, PluginState, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, SessionEventRecord, SessionMeta, StorageEventPayload,
    StoredEvent, SubRunFailure, SubRunFailureCode, SubRunHandoff, SubRunResult, SubRunStorageMode,
    SubagentContextOverrides, TestConnectionResult, ToolOutputStream, format_local_rfc3339,
    plugin::PluginEntry,
};
pub use astrcode_kernel::SubRunStatusView;
pub use astrcode_session_runtime::{
    SessionCatalogEvent, SessionControlStateSnapshot, SessionEventFilterSpec, SessionReplay,
    SessionTranscriptSnapshot, SubRunEventScope, TurnCollaborationSummary, TurnSummary,
};
pub use composer::{ComposerOptionsRequest, ComposerSkillSummary};
pub use errors::ApplicationError;
pub use execution::{ExecutionControl, ProfileResolutionService, RootExecutionRequest};
pub use governance_surface::{
    FreshChildGovernanceInput, GOVERNANCE_APPROVAL_MODE_INHERIT, GOVERNANCE_POLICY_REVISION,
    GovernanceBusyPolicy, GovernanceSurfaceAssembler, ResolvedGovernanceSurface,
    ResumedChildGovernanceInput, RootGovernanceInput, SessionGovernanceInput,
    ToolCollaborationGovernanceContext, build_delegation_metadata, build_fresh_child_contract,
    build_resumed_child_contract, collaboration_policy_context, effective_allowed_tools_for_limits,
};
pub use lifecycle::governance::{
    AppGovernance, ObservabilitySnapshotProvider, RuntimeGovernancePort, RuntimeGovernanceSnapshot,
    RuntimeReloader, SessionInfoProvider,
};
pub use mcp::{
    McpActionSummary, McpConfigScope, McpPort, McpServerStatusSummary, McpServerStatusView,
    McpService, RegisterMcpServerInput,
};
pub use mode::{
    BuiltinModeCatalog, CompiledModeEnvelope, ModeCatalog, ModeSummary, builtin_mode_catalog,
    compile_capability_selector, compile_mode_envelope, compile_mode_envelope_for_child,
    validate_mode_transition,
};
pub use observability::{
    AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot, GovernanceSnapshot,
    OperationMetricsSnapshot, ReloadResult, ReplayMetricsSnapshot, ReplayPath,
    ResolvedRuntimeStatusSummary, RuntimeCapabilitySummary, RuntimeObservabilityCollector,
    RuntimeObservabilitySnapshot, RuntimePluginSummary, SubRunExecutionMetricsSnapshot,
    resolve_runtime_status_summary,
};
pub use ports::{
    AgentKernelPort, AgentSessionPort, AppKernelPort, AppSessionPort, ComposerResolvedSkill,
    ComposerSkillPort,
};
pub use session_plan::{ProjectPlanArchiveDetail, ProjectPlanArchiveSummary};
pub use session_use_cases::summarize_session_meta;
pub use watch::{WatchEvent, WatchPort, WatchService, WatchSource};

/// 唯一业务用例入口。
pub struct App {
    kernel: Arc<dyn AppKernelPort>,
    session_runtime: Arc<dyn AppSessionPort>,
    profiles: Arc<ProfileResolutionService>,
    config_service: Arc<ConfigService>,
    composer_service: Arc<composer::ComposerService>,
    composer_skills: Arc<dyn ComposerSkillPort>,
    governance_surface: Arc<GovernanceSurfaceAssembler>,
    mode_catalog: Arc<ModeCatalog>,
    mcp_service: Arc<mcp::McpService>,
    agent_service: Arc<AgentOrchestrationService>,
}

/// 手动压缩请求的返回结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSessionAccepted {
    /// true 表示压缩被推迟（当前有 turn 正在执行），待 turn 结束后自动执行。
    pub deferred: bool,
}

/// prompt 提交成功后的共享摘要输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptAcceptedSummary {
    pub turn_id: String,
    pub session_id: String,
    pub branched_from_session_id: Option<String>,
    pub accepted_control: Option<ExecutionControl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSkillInvocation {
    pub skill_id: String,
    pub user_prompt: Option<String>,
}

/// 手动 compact 的共享摘要输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSessionSummary {
    pub accepted: bool,
    pub deferred: bool,
    pub message: String,
}

/// session 列表项的共享摘要输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListSummary {
    pub session_id: String,
    pub working_dir: String,
    pub display_name: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub parent_session_id: Option<String>,
    pub parent_storage_seq: Option<u64>,
    pub phase: Phase,
}

/// root agent 执行接受后的共享摘要输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecuteSummary {
    pub accepted: bool,
    pub message: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
}

/// sub-run 状态来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubRunStatusSourceSummary {
    Live,
    Durable,
}

/// sub-run 状态的共享摘要输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRunStatusSummary {
    pub sub_run_id: String,
    pub tool_call_id: Option<String>,
    pub source: SubRunStatusSourceSummary,
    pub agent_id: String,
    pub agent_profile: String,
    pub session_id: String,
    pub child_session_id: Option<String>,
    pub depth: usize,
    pub parent_agent_id: Option<String>,
    pub parent_sub_run_id: Option<String>,
    pub storage_mode: SubRunStorageMode,
    pub lifecycle: AgentLifecycleStatus,
    pub last_turn_outcome: Option<AgentTurnOutcome>,
    pub result: Option<SubRunResult>,
    pub step_count: Option<u32>,
    pub estimated_tokens: Option<u64>,
    pub resolved_overrides: Option<ResolvedSubagentContextOverrides>,
    pub resolved_limits: Option<ResolvedExecutionLimitsSnapshot>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kernel: Arc<dyn AppKernelPort>,
        session_runtime: Arc<dyn AppSessionPort>,
        profiles: Arc<ProfileResolutionService>,
        config_service: Arc<ConfigService>,
        composer_skills: Arc<dyn ComposerSkillPort>,
        governance_surface: Arc<GovernanceSurfaceAssembler>,
        mode_catalog: Arc<ModeCatalog>,
        mcp_service: Arc<mcp::McpService>,
        agent_service: Arc<AgentOrchestrationService>,
    ) -> Self {
        Self {
            kernel,
            session_runtime,
            profiles,
            config_service,
            composer_service: Arc::new(composer::ComposerService::new()),
            composer_skills,
            governance_surface,
            mode_catalog,
            mcp_service,
            agent_service,
        }
    }

    pub fn kernel(&self) -> &Arc<dyn AppKernelPort> {
        &self.kernel
    }

    pub fn session_runtime(&self) -> &Arc<dyn AppSessionPort> {
        &self.session_runtime
    }

    pub fn config(&self) -> &Arc<ConfigService> {
        &self.config_service
    }

    pub fn profiles(&self) -> &Arc<ProfileResolutionService> {
        &self.profiles
    }

    pub fn mcp(&self) -> &Arc<mcp::McpService> {
        &self.mcp_service
    }

    pub fn composer(&self) -> &Arc<composer::ComposerService> {
        &self.composer_service
    }

    pub fn composer_skills(&self) -> &Arc<dyn ComposerSkillPort> {
        &self.composer_skills
    }

    pub fn governance_surface(&self) -> &Arc<GovernanceSurfaceAssembler> {
        &self.governance_surface
    }

    pub fn mode_catalog(&self) -> &Arc<ModeCatalog> {
        &self.mode_catalog
    }

    pub fn agent(&self) -> &Arc<AgentOrchestrationService> {
        &self.agent_service
    }

    pub fn subscribe_catalog(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.session_runtime.subscribe_catalog_events()
    }

    pub async fn execute_root_agent(
        &self,
        request: RootExecutionRequest,
    ) -> Result<ExecutionAccepted, ApplicationError> {
        let runtime = self
            .config_service
            .load_resolved_runtime_config(Some(Path::new(&request.working_dir)))?;
        execution::execute_root_agent(
            self.kernel.as_ref(),
            self.session_runtime.as_ref(),
            &self.profiles,
            self.governance_surface.as_ref(),
            request,
            runtime,
        )
        .await
    }

    pub fn list_global_agent_profiles(&self) -> Result<Vec<AgentProfile>, ApplicationError> {
        Ok(self.profiles.resolve_global()?.as_ref().clone())
    }

    pub fn list_agent_profiles_for_working_dir(
        &self,
        working_dir: &Path,
    ) -> Result<Vec<AgentProfile>, ApplicationError> {
        Ok(self.profiles.resolve(working_dir)?.as_ref().clone())
    }

    pub async fn list_composer_options(
        &self,
        session_id: &str,
        request: ComposerOptionsRequest,
    ) -> Result<Vec<ComposerOption>, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let gateway = self.kernel.gateway();
        let working_dir = self
            .session_runtime
            .get_session_working_dir(session_id)
            .await
            .map_err(ApplicationError::from)?;
        let skill_summaries = self
            .composer_skills
            .list_skill_summaries(Path::new(&working_dir));
        Ok(self
            .composer_service
            .list_options(request, skill_summaries, Some(&gateway)))
    }

    pub async fn get_config(&self) -> Config {
        self.config_service.get_config().await
    }

    pub fn validate_non_empty(
        &self,
        field: &'static str,
        value: &str,
    ) -> Result<(), ApplicationError> {
        if value.trim().is_empty() {
            return Err(ApplicationError::InvalidArgument(format!(
                "field '{}' must not be empty",
                field
            )));
        }
        Ok(())
    }

    pub fn require_permission(
        &self,
        allowed: bool,
        reason: impl Into<String>,
    ) -> Result<(), ApplicationError> {
        if allowed {
            return Ok(());
        }
        Err(ApplicationError::PermissionDenied(reason.into()))
    }
}
