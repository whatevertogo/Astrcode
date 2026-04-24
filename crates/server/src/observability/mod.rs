//! # 可观测性
//!
//! 提供运行时指标快照类型和治理快照能力。
//! 实际的指标收集逻辑由组合根接线。

mod collector;
mod metrics_snapshot;

use std::path::PathBuf;

use astrcode_core::CapabilitySpec;
use astrcode_plugin_host::PluginEntry;
pub use collector::RuntimeObservabilityCollector;
pub use metrics_snapshot::{
    AgentCollaborationScorecardSnapshot, ExecutionDiagnosticsSnapshot, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, RuntimeObservabilitySnapshot, SubRunExecutionMetricsSnapshot,
};

/// 运行时治理快照
///
/// 不依赖 `RuntimeService`，数据来源于运行时治理端口、`SessionRuntime`
/// 和可观测性指标提供者。
#[derive(Debug, Clone)]
pub struct GovernanceSnapshot {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub loaded_session_count: usize,
    pub running_session_ids: Vec<String>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub metrics: RuntimeObservabilitySnapshot,
    pub capabilities: Vec<CapabilitySpec>,
    pub plugins: Vec<PluginEntry>,
}

/// 运行时重载操作的结果。
#[derive(Debug, Clone)]
pub struct ReloadResult {
    /// 重载后的运行时快照
    pub snapshot: GovernanceSnapshot,
    /// 重载完成的时间
    pub reloaded_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentCollaborationScorecardSnapshot, CapabilitySpec, ExecutionDiagnosticsSnapshot,
        OperationMetricsSnapshot, ReplayMetricsSnapshot, SideEffect, Stability,
        SubRunExecutionMetricsSnapshot,
    };
    use astrcode_plugin_host::{
        PluginEntry, PluginHealth, PluginManifest, PluginState, PluginType,
    };
    use serde_json::json;

    use super::{GovernanceSnapshot, RuntimeObservabilitySnapshot};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RuntimeCapabilitySummary {
        name: String,
        kind: String,
        description: String,
        profiles: Vec<String>,
        streaming: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RuntimePluginSummary {
        name: String,
        version: String,
        description: String,
        state: PluginState,
        health: PluginHealth,
        failure_count: u32,
        failure: Option<String>,
        warnings: Vec<String>,
        last_checked_at: Option<String>,
        capabilities: Vec<RuntimeCapabilitySummary>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ResolvedRuntimeStatusSummary {
        runtime_name: String,
        runtime_kind: String,
        loaded_session_count: usize,
        running_session_ids: Vec<String>,
        plugin_search_paths: Vec<String>,
        metrics: RuntimeObservabilitySnapshot,
        capabilities: Vec<RuntimeCapabilitySummary>,
        plugins: Vec<RuntimePluginSummary>,
    }

    fn resolve_runtime_status_summary(
        snapshot: GovernanceSnapshot,
    ) -> ResolvedRuntimeStatusSummary {
        ResolvedRuntimeStatusSummary {
            runtime_name: snapshot.runtime_name,
            runtime_kind: snapshot.runtime_kind,
            loaded_session_count: snapshot.loaded_session_count,
            running_session_ids: snapshot.running_session_ids,
            plugin_search_paths: snapshot
                .plugin_search_paths
                .into_iter()
                .map(|path| path.display().to_string())
                .collect(),
            metrics: snapshot.metrics,
            capabilities: snapshot
                .capabilities
                .into_iter()
                .map(resolve_runtime_capability_summary)
                .collect(),
            plugins: snapshot
                .plugins
                .into_iter()
                .map(resolve_runtime_plugin_summary)
                .collect(),
        }
    }

    fn resolve_runtime_capability_summary(spec: CapabilitySpec) -> RuntimeCapabilitySummary {
        RuntimeCapabilitySummary {
            name: spec.name.to_string(),
            kind: spec.kind.as_str().to_string(),
            description: spec.description,
            profiles: spec.profiles,
            streaming: matches!(
                spec.invocation_mode,
                astrcode_core::InvocationMode::Streaming
            ),
        }
    }

    fn resolve_runtime_plugin_summary(entry: PluginEntry) -> RuntimePluginSummary {
        RuntimePluginSummary {
            name: entry.manifest.name,
            version: entry.manifest.version,
            description: entry.manifest.description,
            state: entry.state,
            health: entry.health,
            failure_count: entry.failure_count,
            failure: entry.failure,
            warnings: entry.warnings,
            last_checked_at: entry.last_checked_at,
            capabilities: entry
                .capabilities
                .into_iter()
                .map(resolve_runtime_capability_summary)
                .collect(),
        }
    }

    fn capability(name: &str, streaming: bool) -> CapabilitySpec {
        let mut builder = CapabilitySpec::builder(name, "tool")
            .description(format!("{name} description"))
            .schema(json!({ "type": "object" }), json!({ "type": "object" }))
            .profiles(["coding"]);
        if streaming {
            builder = builder.invocation_mode(astrcode_core::InvocationMode::Streaming);
        }
        builder
            .side_effect(SideEffect::None)
            .stability(Stability::Stable)
            .build()
            .expect("capability should build")
    }

    fn manifest(name: &str) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: format!("{name} manifest"),
            plugin_type: vec![PluginType::Tool],
            capabilities: Vec::new(),
            executable: Some("plugin.exe".to_string()),
            args: Vec::new(),
            working_dir: None,
            repository: None,
            resources: Vec::new(),
            commands: Vec::new(),
            themes: Vec::new(),
            prompts: Vec::new(),
            providers: Vec::new(),
            skills: Vec::new(),
        }
    }

    fn metrics() -> RuntimeObservabilitySnapshot {
        RuntimeObservabilitySnapshot {
            session_rehydrate: OperationMetricsSnapshot {
                total: 3,
                failures: 1,
                total_duration_ms: 12,
                last_duration_ms: 4,
                max_duration_ms: 7,
            },
            sse_catch_up: ReplayMetricsSnapshot {
                totals: OperationMetricsSnapshot {
                    total: 4,
                    failures: 0,
                    total_duration_ms: 8,
                    last_duration_ms: 2,
                    max_duration_ms: 5,
                },
                cache_hits: 6,
                disk_fallbacks: 1,
                recovered_events: 20,
            },
            turn_execution: OperationMetricsSnapshot {
                total: 5,
                failures: 1,
                total_duration_ms: 15,
                last_duration_ms: 5,
                max_duration_ms: 9,
            },
            subrun_execution: SubRunExecutionMetricsSnapshot {
                total: 6,
                failures: 1,
                completed: 5,
                cancelled: 0,
                independent_session_total: 2,
                total_duration_ms: 18,
                last_duration_ms: 3,
                total_steps: 11,
                last_step_count: 2,
                total_estimated_tokens: 200,
                last_estimated_tokens: 40,
            },
            execution_diagnostics: ExecutionDiagnosticsSnapshot {
                child_spawned: 1,
                child_started_persisted: 1,
                child_terminal_persisted: 1,
                parent_reactivation_requested: 1,
                parent_reactivation_succeeded: 1,
                parent_reactivation_failed: 0,
                lineage_mismatch_parent_agent: 0,
                lineage_mismatch_parent_session: 0,
                lineage_mismatch_child_session: 0,
                lineage_mismatch_descriptor_missing: 0,
                cache_reuse_hits: 2,
                cache_reuse_misses: 1,
                delivery_buffer_queued: 3,
                delivery_buffer_dequeued: 3,
                delivery_buffer_wake_requested: 1,
                delivery_buffer_wake_succeeded: 1,
                delivery_buffer_wake_failed: 0,
            },
            agent_collaboration: AgentCollaborationScorecardSnapshot {
                total_facts: 2,
                spawn_accepted: 1,
                spawn_rejected: 1,
                send_reused: 0,
                send_queued: 1,
                send_rejected: 0,
                observe_calls: 1,
                observe_rejected: 0,
                observe_followed_by_action: 1,
                close_calls: 0,
                close_rejected: 0,
                delivery_delivered: 1,
                delivery_consumed: 1,
                delivery_replayed: 0,
                orphan_child_count: 0,
                child_reuse_ratio_bps: Some(5000),
                observe_to_action_ratio_bps: Some(10000),
                spawn_to_delivery_ratio_bps: Some(10000),
                orphan_child_ratio_bps: Some(0),
                avg_delivery_latency_ms: Some(12),
                max_delivery_latency_ms: Some(12),
            },
        }
    }

    #[test]
    fn resolve_runtime_status_summary_projects_snapshot_inputs() {
        let snapshot = GovernanceSnapshot {
            runtime_name: "astrcode".to_string(),
            runtime_kind: "desktop".to_string(),
            loaded_session_count: 2,
            running_session_ids: vec!["session-a".to_string()],
            plugin_search_paths: vec!["C:/plugins".into()],
            metrics: metrics(),
            capabilities: vec![capability("tool.repo.inspect", true)],
            plugins: vec![PluginEntry {
                manifest: manifest("repo-plugin"),
                state: PluginState::Initialized,
                health: PluginHealth::Healthy,
                failure_count: 0,
                capabilities: vec![capability("tool.repo.inspect", false)],
                failure: None,
                warnings: vec!["skill warning".to_string()],
                last_checked_at: Some("2026-04-16T12:00:00+08:00".to_string()),
            }],
        };

        let summary = resolve_runtime_status_summary(snapshot);

        assert_eq!(summary.runtime_name, "astrcode");
        assert_eq!(summary.plugin_search_paths, vec!["C:/plugins".to_string()]);
        assert_eq!(summary.capabilities.len(), 1);
        assert!(summary.capabilities[0].streaming);
        assert_eq!(summary.capabilities[0].kind, "tool");
        assert_eq!(summary.plugins.len(), 1);
        assert_eq!(summary.plugins[0].name, "repo-plugin");
        assert_eq!(summary.plugins[0].capabilities.len(), 1);
        assert!(!summary.plugins[0].capabilities[0].streaming);
        assert_eq!(
            summary.plugins[0].last_checked_at.as_deref(),
            Some("2026-04-16T12:00:00+08:00")
        );
        assert_eq!(summary.metrics.session_rehydrate.total, 3);
    }
}
