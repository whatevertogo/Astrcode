use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use astrcode_core::{
    AgentEventContext, AstrError, CancelToken, CapabilityContext, CapabilityExecutionResult,
    ExecutionOwner, InvocationKind, InvocationMode, Result, SessionId, TurnId,
    mode::{BoundModeToolContractSnapshot, ModeId},
};
use astrcode_protocol::plugin::{
    CapabilityWireDescriptor, ErrorPayload, EventMessage, EventPhase, InitializeResultData,
    InvokeMessage, ResultMessage, SkillDescriptor,
};

use super::{
    ActivePluginRuntimeCatalog, BuiltinCapabilityExecutor, BuiltinCapabilityExecutorRegistry,
    NegotiatedPluginCatalog, PluginCapabilityBinding, PluginCapabilityDispatchKind,
    PluginCapabilityDispatchOutcome, PluginCapabilityDispatchReadiness,
    PluginCapabilityDispatcherSet, PluginCapabilityHttpDispatch, PluginCapabilityHttpDispatcher,
    PluginCapabilityHttpDispatcherRegistry, PluginCapabilityInvocationPlan,
    PluginCapabilityProtocolDispatch, PluginCapabilityProtocolDispatcher,
    PluginCapabilityProtocolDispatcherRegistry, PluginCapabilityProtocolExecution,
    PluginCapabilityProtocolTransport, PluginHost, PluginRuntimeHandleRef,
    TransportBackedProtocolDispatcher,
};
use crate::{
    PluginDescriptor, PluginLoader, PluginSourceKind,
    backend::{PluginBackendHealth, PluginBackendKind, PluginProcessStatus},
};

fn unique_temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough")
        .as_nanos();
    std::env::temp_dir().join(format!("astrcode-plugin-host-host-{name}-{suffix}"))
}

fn shell_command_with_args(script: &str) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
        (command, vec!["/C".to_string(), script.to_string()])
    }
    #[cfg(not(windows))]
    {
        (
            "/bin/sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
        )
    }
}

fn node_protocol_command() -> (String, Vec<String>) {
    let script = r#"
const readline = require('node:readline');
const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on('line', (line) => {
  const msg = JSON.parse(line);
  if (msg.type === 'initialize') {
    console.log(JSON.stringify({
      type: 'result',
      id: msg.id,
      kind: 'initialize',
      success: true,
      output: {
        protocolVersion: '5',
        peer: {
          id: 'fixture-worker',
          name: 'fixture-worker',
          role: 'worker',
          version: '0.1.0',
          supportedProfiles: ['coding'],
          metadata: { fixture: true }
        },
        capabilities: [],
        handlers: [],
        profiles: [{
          name: 'coding',
          version: '1',
          description: 'coding',
          contextSchema: null,
          metadata: null
        }],
        skills: [],
        modes: [],
        metadata: null
      },
      metadata: null
    }));
    return;
  }
  if (msg.type === 'invoke' && msg.stream) {
    console.log(JSON.stringify({
      type: 'event',
      id: msg.id,
      phase: 'started',
      event: 'tool.started',
      payload: { capability: msg.capability },
      seq: 0
    }));
    console.log(JSON.stringify({
      type: 'event',
      id: msg.id,
      phase: 'delta',
      event: 'tool.delta',
      payload: { chunk: 1 },
      seq: 1
    }));
    console.log(JSON.stringify({
      type: 'event',
      id: msg.id,
      phase: 'completed',
      event: 'tool.completed',
      payload: { ok: true },
      seq: 2
    }));
    return;
  }
  if (msg.type === 'invoke') {
    console.log(JSON.stringify({
      type: 'result',
      id: msg.id,
      kind: 'tool_result',
      success: true,
      output: { echoed: msg.input },
      metadata: { transport: 'node-fixture' }
    }));
  }
});
"#;
    (
        "node".to_string(),
        vec!["-e".to_string(), script.to_string()],
    )
}

fn node_protocol_command_exit_after_initialize() -> (String, Vec<String>) {
    let script = r#"
const readline = require('node:readline');
const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
let initialized = false;
rl.on('line', (line) => {
  const msg = JSON.parse(line);
  if (msg.type === 'initialize' && !initialized) {
    initialized = true;
    console.log(JSON.stringify({
      type: 'result',
      id: msg.id,
      kind: 'initialize',
      success: true,
      output: {
        protocolVersion: '5',
        peer: {
          id: 'fixture-worker',
          name: 'fixture-worker',
          role: 'worker',
          version: '0.1.0',
          supportedProfiles: ['coding'],
          metadata: { fixture: true }
        },
        capabilities: [],
        handlers: [],
        profiles: [{
          name: 'coding',
          version: '1',
          description: 'coding',
          contextSchema: null,
          metadata: null
        }],
        skills: [],
        modes: [],
        metadata: null
      },
      metadata: null
    }));
    setImmediate(() => process.exit(0));
  }
});
"#;
    (
        "node".to_string(),
        vec!["-e".to_string(), script.to_string()],
    )
}

fn sample_capability_context() -> CapabilityContext {
    CapabilityContext {
        request_id: Some("req-1".to_string()),
        trace_id: Some("trace-1".to_string()),
        session_id: SessionId::from("session-1"),
        working_dir: PathBuf::from("D:/repo"),
        cancel: CancelToken::new(),
        turn_id: Some("turn-1".to_string()),
        agent: AgentEventContext::root_execution("agent-1", "coding"),
        current_mode_id: ModeId::from("coding"),
        bound_mode_tool_contract: Some(BoundModeToolContractSnapshot {
            mode_id: ModeId::from("coding"),
            artifact: None,
            exit_gate: None,
        }),
        execution_owner: Some(ExecutionOwner::root(
            SessionId::from("session-1"),
            TurnId::from("turn-1"),
            InvocationKind::RootExecution,
        )),
        profile: "coding".to_string(),
        profile_context: serde_json::json!({ "cwd": "D:/repo" }),
        metadata: serde_json::json!({ "source": "test" }),
        tool_output_sender: None,
        event_sink: None,
    }
}

struct StaticBuiltinExecutor;

impl BuiltinCapabilityExecutor for StaticBuiltinExecutor {
    fn execute(
        &self,
        plan: &PluginCapabilityInvocationPlan,
        _ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        Ok(CapabilityExecutionResult::ok(
            plan.binding.capability.name.to_string(),
            serde_json::json!({
                "executedBy": plan.binding.plugin_id,
                "input": plan.payload,
            }),
        ))
    }
}

struct StaticProtocolDispatcher {
    execution: PluginCapabilityProtocolExecution,
}

impl PluginCapabilityProtocolDispatcher for StaticProtocolDispatcher {
    fn dispatch(
        &self,
        _dispatch: &PluginCapabilityProtocolDispatch,
    ) -> Result<PluginCapabilityProtocolExecution> {
        Ok(self.execution.clone())
    }
}

struct StaticHttpDispatcher;

impl PluginCapabilityHttpDispatcher for StaticHttpDispatcher {
    fn dispatch(
        &self,
        dispatch: &PluginCapabilityHttpDispatch,
    ) -> Result<CapabilityExecutionResult> {
        Ok(CapabilityExecutionResult::ok(
            dispatch.target.plan.binding.capability.name.to_string(),
            serde_json::json!({
                "transport": "http",
                "pluginId": dispatch.target.plan.binding.plugin_id,
            }),
        ))
    }
}

#[derive(Clone)]
struct FakeProtocolTransport {
    unary: Option<ResultMessage>,
    stream: Vec<EventMessage>,
}

impl PluginCapabilityProtocolTransport for FakeProtocolTransport {
    fn invoke_unary(&self, _dispatch: &PluginCapabilityProtocolDispatch) -> Result<ResultMessage> {
        self.unary.clone().ok_or_else(|| {
            AstrError::Validation("fake protocol transport missing unary response".to_string())
        })
    }

    fn invoke_stream(
        &self,
        _dispatch: &PluginCapabilityProtocolDispatch,
    ) -> Result<Vec<EventMessage>> {
        Ok(self.stream.clone())
    }
}

#[test]
fn reload_from_loader_promotes_discovered_snapshot() {
    let root = unique_temp_dir("reload");
    fs::create_dir_all(&root).expect("temp dir should create");
    fs::write(
        root.join("repo-inspector.toml"),
        r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "./bin/repo-inspector"
args = ["--stdio"]
working_dir = "."
repository = "https://example.com/repo-inspector"
"#,
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let snapshot = host
        .reload_from_loader(&PluginLoader {
            search_paths: vec![root.clone()],
        })
        .expect("reload should succeed");

    assert_eq!(snapshot.plugin_ids, vec!["repo-inspector".to_string()]);
    assert_eq!(
        host.active_snapshot()
            .expect("active snapshot should exist")
            .plugin_ids,
        vec!["repo-inspector".to_string()]
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reload_with_external_backends_returns_snapshot_and_external_handles() {
    let root = unique_temp_dir("reload-with-backends");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}
working_dir = "."
repository = "https://example.com/repo-inspector"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let mut reload = host
        .reload_with_external_backends(&PluginLoader {
            search_paths: vec![root.clone()],
        })
        .await
        .expect("reload with backends should succeed");

    assert_eq!(
        reload.snapshot.plugin_ids,
        vec!["repo-inspector".to_string()]
    );
    assert_eq!(reload.descriptors.len(), 1);
    assert!(reload.builtin_backends.is_empty());
    assert_eq!(reload.external_backends.len(), 1);
    assert_eq!(
        reload.resources.plugin_ids,
        vec!["repo-inspector".to_string()]
    );
    assert_eq!(
        reload.negotiated_plugins.plugin_ids,
        vec!["repo-inspector".to_string()]
    );
    assert_eq!(
        reload.runtime_catalog.plugin_ids,
        vec!["repo-inspector".to_string()]
    );
    assert_eq!(reload.runtime_catalog.entries.len(), 1);
    assert_eq!(
        reload.runtime_catalog.snapshot_id,
        reload.snapshot.snapshot_id
    );
    assert_eq!(reload.runtime_catalog.revision, reload.snapshot.revision);
    assert_eq!(
        reload.runtime_catalog.entries[0].plugin_id,
        "repo-inspector"
    );
    assert!(reload.runtime_catalog.entries[0].has_external_backend);
    assert!(!reload.runtime_catalog.entries[0].has_builtin_backend);
    assert!(reload.runtime_catalog.entries[0].has_runtime_handle);
    assert_eq!(
        reload.runtime_catalog.entries[0].backend_health,
        Some(PluginBackendHealth::Healthy)
    );
    assert!(
        reload.runtime_catalog.entries[0]
            .backend_health_message
            .is_none()
    );
    assert_eq!(
        reload.external_backends[0]
            .protocol_state()
            .expect("protocol state should be attached")
            .local_initialize
            .peer
            .id,
        "plugin-host"
    );
    assert_eq!(
        reload.negotiated_plugins.remote_plugins[0].local_protocol_version,
        "5"
    );
    assert_eq!(
        reload.runtime_catalog.negotiated_plugins.remote_plugins[0].local_protocol_version,
        "5"
    );
    assert_eq!(
        reload.runtime_catalog.entries[0]
            .local_protocol_version
            .as_deref(),
        Some("5")
    );
    assert!(reload.negotiated_plugins.remote_plugins[0].remote.is_none());
    let reports = host
        .external_backend_health_reports(&mut reload.external_backends)
        .expect("health reports should be readable");
    assert_eq!(reports[0].health, PluginBackendHealth::Healthy);

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reload_can_record_remote_initialize_and_refresh_catalog() {
    let root = unique_temp_dir("reload-with-remote-handshake");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}
working_dir = "."
repository = "https://example.com/repo-inspector"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let mut reload = host
        .reload_with_external_backends(&PluginLoader {
            search_paths: vec![root.clone()],
        })
        .await
        .expect("reload with backends should succeed");

    reload
        .record_remote_initialize(
            "repo-inspector",
            InitializeResultData {
                protocol_version: "5".to_string(),
                peer: reload.external_backends[0]
                    .protocol_state()
                    .expect("protocol state should exist")
                    .local_initialize
                    .peer
                    .clone(),
                capabilities: Vec::new(),
                handlers: Vec::new(),
                profiles: vec![crate::default_profiles()[0].clone()],
                skills: vec![SkillDescriptor {
                    name: "skill.review".to_string(),
                    description: "review".to_string(),
                    guide: "guide".to_string(),
                    allowed_tools: Vec::new(),
                    assets: Vec::new(),
                    metadata: serde_json::Value::Null,
                }],
                modes: Vec::new(),
                metadata: serde_json::Value::Null,
            },
        )
        .expect("record remote initialize should succeed");

    let negotiated = &reload.negotiated_plugins.remote_plugins[0];
    assert_eq!(negotiated.plugin_id, "repo-inspector");
    assert_eq!(negotiated.local_protocol_version, "5");
    assert_eq!(
        negotiated
            .remote
            .as_ref()
            .expect("remote summary should exist")
            .skill_ids,
        vec!["skill.review".to_string()]
    );
    assert_eq!(
        reload.runtime_catalog.negotiated_plugins.remote_plugins[0]
            .remote
            .as_ref()
            .expect("runtime catalog remote summary should exist")
            .skill_ids,
        vec!["skill.review".to_string()]
    );
    assert_eq!(
        reload.runtime_catalog.entries[0]
            .remote
            .as_ref()
            .expect("runtime entry remote summary should exist")
            .skill_ids,
        vec!["skill.review".to_string()]
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn backend_plans_cover_builtin_and_process_descriptors() {
    let host = PluginHost::new();
    let builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some("/bin/repo-inspector".to_string());
    external.launch_args = vec!["--stdio".to_string()];

    let plans = host
        .backend_plans(&[builtin, external])
        .expect("backend plans should build");

    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].backend_kind, PluginBackendKind::InProcess);
    assert_eq!(plans[1].backend_kind, PluginBackendKind::Process);
    assert_eq!(host.local_peer().id, "plugin-host");
}

#[tokio::test]
async fn start_external_process_backends_only_launches_external_entries() {
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();

    let host = PluginHost::new();
    let builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;

    let plans = host
        .backend_plans(&[builtin, external])
        .expect("backend plans should build");
    let mut backends = host
        .start_external_process_backends(&plans)
        .await
        .expect("external backends should start");

    assert_eq!(backends.len(), 1);
    assert!(backends[0].protocol_state().is_some());
    let status = backends[0].status().expect("status should be readable");
    assert_eq!(
        status,
        PluginProcessStatus {
            running: true,
            exit_code: None
        }
    );
    let reports = host
        .external_backend_health_reports(&mut backends)
        .expect("health reports should be readable");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].health, PluginBackendHealth::Healthy);

    for backend in &mut backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn reload_from_descriptors_unifies_builtin_and_external_entries() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.commands.push(crate::descriptor::CommandDescriptor {
        command_id: "review".to_string(),
        entry_ref: ".codex/commands/review.md".to_string(),
    });
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;

    let mut reload = host
        .reload_from_descriptors(vec![builtin, external])
        .await
        .expect("mixed reload should succeed");

    assert_eq!(
        reload.runtime_catalog.plugin_ids,
        vec!["core-tools".to_string(), "repo-inspector".to_string()]
    );
    assert_eq!(reload.runtime_catalog.entries.len(), 2);
    assert!(!reload.runtime_catalog.entries[0].has_external_backend);
    assert!(reload.runtime_catalog.entries[0].has_builtin_backend);
    assert!(reload.runtime_catalog.entries[0].has_runtime_handle);
    assert!(reload.runtime_catalog.entries[1].has_external_backend);
    assert!(!reload.runtime_catalog.entries[1].has_builtin_backend);
    assert!(reload.runtime_catalog.entries[1].has_runtime_handle);
    assert_eq!(
        reload.runtime_catalog.entries[0].backend_health,
        Some(PluginBackendHealth::Healthy)
    );
    assert_eq!(
        reload.runtime_catalog.entries[1].backend_health,
        Some(PluginBackendHealth::Healthy)
    );
    assert_eq!(
        reload.runtime_catalog.entries[0].command_ids,
        vec!["review".to_string()]
    );
    assert_eq!(
        reload.runtime_catalog.entries[1]
            .local_protocol_version
            .as_deref(),
        Some("5")
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn reload_with_builtin_and_loader_unifies_mixed_sources() {
    let root = unique_temp_dir("reload-builtin-and-loader");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}
working_dir = "."
repository = "https://example.com/repo-inspector"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.commands.push(crate::descriptor::CommandDescriptor {
        command_id: "review".to_string(),
        entry_ref: ".codex/commands/review.md".to_string(),
    });

    let mut reload = host
        .reload_with_builtin_and_loader(
            vec![builtin],
            &PluginLoader {
                search_paths: vec![root.clone()],
            },
        )
        .await
        .expect("mixed builtin + loader reload should succeed");

    assert_eq!(
        reload.runtime_catalog.plugin_ids,
        vec!["core-tools".to_string(), "repo-inspector".to_string()]
    );
    assert_eq!(reload.runtime_catalog.entries.len(), 2);
    assert_eq!(reload.runtime_catalog.entries[0].plugin_id, "core-tools");
    assert!(!reload.runtime_catalog.entries[0].has_external_backend);
    assert!(reload.runtime_catalog.entries[0].has_builtin_backend);
    assert!(reload.runtime_catalog.entries[0].has_runtime_handle);
    assert_eq!(
        reload.runtime_catalog.entries[1].plugin_id,
        "repo-inspector"
    );
    assert!(reload.runtime_catalog.entries[1].has_external_backend);
    assert!(!reload.runtime_catalog.entries[1].has_builtin_backend);
    assert!(reload.runtime_catalog.entries[1].has_runtime_handle);
    assert_eq!(
        reload.runtime_catalog.entries[1].backend_health,
        Some(PluginBackendHealth::Healthy)
    );
    assert_eq!(
        reload.runtime_catalog.entries[1]
            .local_protocol_version
            .as_deref(),
        Some("5")
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reload_with_builtin_and_loader_merges_resource_batches_through_single_catalog_owner() {
    let root = unique_temp_dir("reload-builtin-loader-resources");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}

[[commands]]
id = "external-review"
entry_ref = ".codex/commands/external-review.md"

[[prompts]]
id = "prompt.external-review"
body = "Review external repository"

[[skills]]
id = "skill.external-review"
entry_ref = ".codex/skills/review/SKILL.md"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.commands.push(crate::descriptor::CommandDescriptor {
        command_id: "review".to_string(),
        entry_ref: ".codex/commands/review.md".to_string(),
    });

    let mut reload = host
        .reload_with_builtin_and_loader(
            vec![builtin],
            &PluginLoader {
                search_paths: vec![root.clone()],
            },
        )
        .await
        .expect("mixed builtin + loader reload should succeed");

    assert_eq!(
        reload.resources.plugin_ids,
        vec!["core-tools".to_string(), "repo-inspector".to_string()]
    );
    assert_eq!(reload.resources.commands.len(), 2);
    assert_eq!(reload.resources.prompts.len(), 1);
    assert_eq!(reload.resources.skills.len(), 1);
    assert_eq!(
        reload.runtime_catalog.command_ids,
        vec!["review".to_string(), "external-review".to_string()]
    );
    assert_eq!(
        reload.runtime_catalog.prompt_ids,
        vec!["prompt.external-review".to_string()]
    );
    assert_eq!(
        reload.runtime_catalog.skill_ids,
        vec!["skill.external-review".to_string()]
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reload_with_builtin_loader_and_capabilities_propagates_local_capabilities() {
    let root = unique_temp_dir("reload-builtin-loader-with-capabilities");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}
working_dir = "."
repository = "https://example.com/repo-inspector"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    let capabilities = vec![
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .tags(["io", "read"])
            .build()
            .expect("capability should build"),
    ];

    let mut reload = host
        .reload_with_builtin_loader_and_capabilities(
            vec![builtin],
            &PluginLoader {
                search_paths: vec![root.clone()],
            },
            &capabilities,
        )
        .await
        .expect("mixed builtin + loader reload with capabilities should succeed");

    assert_eq!(reload.external_backends.len(), 1);
    let protocol_state = reload.external_backends[0]
        .protocol_state()
        .expect("protocol state should be attached");
    assert_eq!(protocol_state.local_initialize.capabilities, capabilities);
    assert_eq!(protocol_state.local_initialize.peer.id, "plugin-host");

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn negotiated_plugin_catalog_reflects_backend_protocol_state() {
    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    let (command, args) = shell_command_with_args("ping 127.0.0.1 -n 5 >nul");
    external.launch_command = Some(command);
    external.launch_args = args;

    let plans = host
        .backend_plans(&[external])
        .expect("backend plans should build");
    let mut backends = tokio::runtime::Runtime::new()
        .expect("runtime should build")
        .block_on(host.start_external_process_backends(&plans))
        .expect("external backends should start");

    let catalog = NegotiatedPluginCatalog::from_external_backends(&backends);
    assert_eq!(catalog.plugin_ids, vec!["repo-inspector".to_string()]);
    assert_eq!(catalog.remote_plugins.len(), 1);
    assert_eq!(catalog.remote_plugins[0].plugin_id, "repo-inspector");
    assert_eq!(catalog.remote_plugins[0].local_protocol_version, "5");
    assert!(catalog.remote_plugins[0].remote.is_none());

    tokio::runtime::Runtime::new()
        .expect("runtime should build")
        .block_on(async {
            for backend in &mut backends {
                backend.shutdown().await.expect("shutdown should succeed");
            }
        });
}

#[test]
fn active_runtime_catalog_collects_runtime_facts() {
    let mut descriptor = PluginDescriptor::builtin("core-tools", "Core Tools");
    descriptor
        .commands
        .push(crate::descriptor::CommandDescriptor {
            command_id: "review".to_string(),
            entry_ref: ".codex/commands/review.md".to_string(),
        });
    descriptor
        .prompts
        .push(crate::descriptor::PromptDescriptor {
            prompt_id: "prompt.review".to_string(),
            body: "review".to_string(),
        });

    let snapshot =
        crate::PluginActiveSnapshot::from_descriptors(7, "snapshot-7", &[descriptor.clone()]);
    let resources = crate::ResourceCatalog::from_descriptors(&[descriptor.clone()]);
    let reload = super::PluginHostReload {
        descriptors: vec![descriptor],
        snapshot,
        builtin_backends: Vec::new(),
        external_backends: Vec::new(),
        resources,
        backend_health: super::ExternalBackendHealthCatalog::default(),
        negotiated_plugins: NegotiatedPluginCatalog::default(),
        runtime_catalog: ActivePluginRuntimeCatalog {
            snapshot_id: String::new(),
            revision: 0,
            plugin_ids: Vec::new(),
            entries: Vec::new(),
            tool_names: Vec::new(),
            hook_ids: Vec::new(),
            provider_ids: Vec::new(),
            resource_ids: Vec::new(),
            command_ids: Vec::new(),
            theme_ids: Vec::new(),
            prompt_ids: Vec::new(),
            skill_ids: Vec::new(),
            negotiated_plugins: NegotiatedPluginCatalog::default(),
        },
    };

    let catalog = ActivePluginRuntimeCatalog::from_reload(&reload);
    assert_eq!(catalog.snapshot_id, "snapshot-7");
    assert_eq!(catalog.revision, 7);
    assert_eq!(catalog.plugin_ids, vec!["core-tools".to_string()]);
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(catalog.entries[0].plugin_id, "core-tools");
    assert!(!catalog.entries[0].has_external_backend);
    assert!(!catalog.entries[0].has_builtin_backend);
    assert!(!catalog.entries[0].has_runtime_handle);
    assert_eq!(catalog.command_ids, vec!["review".to_string()]);
    assert_eq!(catalog.prompt_ids, vec!["prompt.review".to_string()]);
}

#[test]
fn active_runtime_catalog_resolves_plugin_ownership() {
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("tool capability should build"),
    );
    builtin.hooks.push(crate::descriptor::HookDescriptor {
        hook_id: "tool_call".to_string(),
        event: "tool_call".to_string(),
        ..Default::default()
    });
    builtin.commands.push(crate::descriptor::CommandDescriptor {
        command_id: "review".to_string(),
        entry_ref: ".codex/commands/review.md".to_string(),
    });
    builtin.prompts.push(crate::descriptor::PromptDescriptor {
        prompt_id: "prompt.review".to_string(),
        body: "review".to_string(),
    });
    builtin.skills.push(crate::descriptor::SkillDescriptor {
        skill_id: "skill.review".to_string(),
        entry_ref: ".codex/skills/review/SKILL.md".to_string(),
    });

    let mut external = PluginDescriptor::builtin("corp-provider", "Corp Provider");
    external.source_kind = PluginSourceKind::Process;
    external
        .providers
        .push(crate::descriptor::ProviderDescriptor {
            provider_id: "corp-ai".to_string(),
            api_kind: "openai-compatible".to_string(),
        });

    let snapshot = crate::PluginActiveSnapshot::from_descriptors(
        8,
        "snapshot-8",
        &[builtin.clone(), external.clone()],
    );
    let resources = crate::ResourceCatalog::from_descriptors(&[builtin.clone(), external.clone()]);
    let reload = super::PluginHostReload {
        descriptors: vec![builtin, external],
        snapshot,
        builtin_backends: Vec::new(),
        external_backends: Vec::new(),
        resources,
        backend_health: super::ExternalBackendHealthCatalog::default(),
        negotiated_plugins: NegotiatedPluginCatalog::default(),
        runtime_catalog: ActivePluginRuntimeCatalog {
            snapshot_id: String::new(),
            revision: 0,
            plugin_ids: Vec::new(),
            entries: Vec::new(),
            tool_names: Vec::new(),
            hook_ids: Vec::new(),
            provider_ids: Vec::new(),
            resource_ids: Vec::new(),
            command_ids: Vec::new(),
            theme_ids: Vec::new(),
            prompt_ids: Vec::new(),
            skill_ids: Vec::new(),
            negotiated_plugins: NegotiatedPluginCatalog::default(),
        },
    };

    let catalog = ActivePluginRuntimeCatalog::from_reload(&reload);
    assert_eq!(
        catalog
            .entry("core-tools")
            .expect("entry should exist")
            .display_name,
        "Core Tools"
    );
    assert_eq!(catalog.enabled_entries().len(), 2);
    assert_eq!(
        catalog
            .tool_owner("tool.read")
            .expect("tool owner should exist")
            .plugin_id,
        "core-tools"
    );
    assert_eq!(
        catalog
            .hook_owner("tool_call")
            .expect("hook owner should exist")
            .plugin_id,
        "core-tools"
    );
    assert_eq!(
        catalog
            .command_owner("review")
            .expect("command owner should exist")
            .plugin_id,
        "core-tools"
    );
    assert_eq!(
        catalog
            .prompt_owner("prompt.review")
            .expect("prompt owner should exist")
            .plugin_id,
        "core-tools"
    );
    assert_eq!(
        catalog
            .skill_owner("skill.review")
            .expect("skill owner should exist")
            .plugin_id,
        "core-tools"
    );
    assert_eq!(
        catalog
            .provider_owner("corp-ai")
            .expect("provider owner should exist")
            .plugin_id,
        "corp-provider"
    );
}

#[test]
fn plugin_host_reload_resolves_descriptors_by_contribution_id() {
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("tool capability should build"),
    );
    builtin.hooks.push(crate::descriptor::HookDescriptor {
        hook_id: "tool_call".to_string(),
        event: "tool_call".to_string(),
        ..Default::default()
    });
    builtin
        .resources
        .push(crate::descriptor::ResourceDescriptor {
            resource_id: "skill-dir".to_string(),
            kind: "skill".to_string(),
            locator: ".codex/skills".to_string(),
        });
    builtin.commands.push(crate::descriptor::CommandDescriptor {
        command_id: "review".to_string(),
        entry_ref: ".codex/commands/review.md".to_string(),
    });
    builtin.themes.push(crate::descriptor::ThemeDescriptor {
        theme_id: "light".to_string(),
    });
    builtin.prompts.push(crate::descriptor::PromptDescriptor {
        prompt_id: "prompt.review".to_string(),
        body: "review".to_string(),
    });
    builtin.skills.push(crate::descriptor::SkillDescriptor {
        skill_id: "skill.review".to_string(),
        entry_ref: ".codex/skills/review/SKILL.md".to_string(),
    });

    let mut external = PluginDescriptor::builtin("corp-provider", "Corp Provider");
    external.source_kind = PluginSourceKind::Process;
    external
        .providers
        .push(crate::descriptor::ProviderDescriptor {
            provider_id: "corp-ai".to_string(),
            api_kind: "openai-compatible".to_string(),
        });

    let snapshot = crate::PluginActiveSnapshot::from_descriptors(
        9,
        "snapshot-9",
        &[builtin.clone(), external.clone()],
    );
    let resources = crate::ResourceCatalog::from_descriptors(&[builtin.clone(), external.clone()]);
    let reload = super::PluginHostReload {
        descriptors: vec![builtin, external],
        snapshot,
        builtin_backends: Vec::new(),
        external_backends: Vec::new(),
        resources,
        backend_health: super::ExternalBackendHealthCatalog::default(),
        negotiated_plugins: NegotiatedPluginCatalog::default(),
        runtime_catalog: ActivePluginRuntimeCatalog {
            snapshot_id: String::new(),
            revision: 0,
            plugin_ids: Vec::new(),
            entries: Vec::new(),
            tool_names: Vec::new(),
            hook_ids: Vec::new(),
            provider_ids: Vec::new(),
            resource_ids: Vec::new(),
            command_ids: Vec::new(),
            theme_ids: Vec::new(),
            prompt_ids: Vec::new(),
            skill_ids: Vec::new(),
            negotiated_plugins: NegotiatedPluginCatalog::default(),
        },
    };

    assert_eq!(
        reload
            .plugin_descriptor("core-tools")
            .expect("plugin should exist")
            .display_name,
        "Core Tools"
    );
    assert_eq!(
        reload
            .tool_descriptor("tool.read")
            .expect("tool descriptor should exist")
            .0
            .plugin_id,
        "core-tools"
    );
    assert_eq!(
        reload
            .hook_descriptor("tool_call")
            .expect("hook descriptor should exist")
            .1
            .event,
        "tool_call"
    );
    assert_eq!(
        reload
            .provider_descriptor("corp-ai")
            .expect("provider descriptor should exist")
            .0
            .plugin_id,
        "corp-provider"
    );
    assert_eq!(
        reload
            .resource_descriptor("skill-dir")
            .expect("resource descriptor should exist")
            .1
            .locator,
        ".codex/skills"
    );
    assert_eq!(
        reload
            .command_descriptor("review")
            .expect("command descriptor should exist")
            .1
            .entry_ref,
        ".codex/commands/review.md"
    );
    assert_eq!(
        reload
            .theme_descriptor("light")
            .expect("theme descriptor should exist")
            .0
            .plugin_id,
        "core-tools"
    );
    assert_eq!(
        reload
            .prompt_descriptor("prompt.review")
            .expect("prompt descriptor should exist")
            .1
            .body,
        "review"
    );
    assert_eq!(
        reload
            .skill_descriptor("skill.review")
            .expect("skill descriptor should exist")
            .1
            .entry_ref,
        ".codex/skills/review/SKILL.md"
    );
}

#[tokio::test]
async fn reload_can_refresh_backend_health_into_runtime_catalog() {
    let root = unique_temp_dir("reload-refresh-backend-health");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}
working_dir = "."
repository = "https://example.com/repo-inspector"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let mut reload = host
        .reload_with_external_backends(&PluginLoader {
            search_paths: vec![root.clone()],
        })
        .await
        .expect("reload with backends should succeed");

    assert_eq!(
        reload.runtime_catalog.entries[0].backend_health,
        Some(PluginBackendHealth::Healthy)
    );

    reload
        .refresh_external_backend_health(&host)
        .expect("refresh external backend health should succeed");
    assert_eq!(
        reload.runtime_catalog.entries[0].backend_health,
        Some(PluginBackendHealth::Healthy)
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reload_exposes_unified_runtime_handles_for_builtin_and_external() {
    let root = unique_temp_dir("reload-unified-runtime-handles");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}
working_dir = "."
repository = "https://example.com/repo-inspector"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    let mut reload = host
        .reload_with_builtin_and_loader(
            vec![builtin],
            &PluginLoader {
                search_paths: vec![root.clone()],
            },
        )
        .await
        .expect("mixed reload should succeed");

    match reload
        .runtime_handle("core-tools")
        .expect("builtin runtime handle should exist")
    {
        PluginRuntimeHandleRef::Builtin(handle) => {
            assert_eq!(handle.plugin_id, "core-tools");
            assert!(handle.started_at_ms > 0);
        },
        PluginRuntimeHandleRef::External(_) => {
            panic!("builtin plugin should not resolve to external handle");
        },
    }

    match reload
        .runtime_handle("repo-inspector")
        .expect("external runtime handle should exist")
    {
        PluginRuntimeHandleRef::Builtin(_) => {
            panic!("external plugin should not resolve to builtin handle");
        },
        PluginRuntimeHandleRef::External(handle) => {
            assert_eq!(handle.plugin_id, "repo-inspector");
            assert!(handle.started_at_ms > 0);
        },
    }

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reload_exposes_runtime_handle_snapshots_without_raw_handle_access() {
    let root = unique_temp_dir("reload-runtime-handle-snapshots");
    fs::create_dir_all(&root).expect("temp dir should create");
    #[cfg(windows)]
    let executable = "cmd.exe";
    #[cfg(not(windows))]
    let executable = "/bin/sh";
    #[cfg(windows)]
    let args = r#"args = ["/C", "ping 127.0.0.1 -n 5 >nul"]"#;
    #[cfg(not(windows))]
    let args = r#"args = ["-c", "sleep 2"]"#;
    fs::write(
        root.join("repo-inspector.toml"),
        format!(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "{executable}"
{args}
working_dir = "."
repository = "https://example.com/repo-inspector"
"#
        ),
    )
    .expect("manifest should write");

    let host = PluginHost::new();
    let builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    let mut reload = host
        .reload_with_builtin_and_loader(
            vec![builtin],
            &PluginLoader {
                search_paths: vec![root.clone()],
            },
        )
        .await
        .expect("mixed reload should succeed");

    let builtin_snapshot = reload
        .runtime_handle_snapshot("core-tools")
        .expect("builtin snapshot should exist");
    assert_eq!(builtin_snapshot.backend_kind, PluginBackendKind::InProcess);
    assert_eq!(builtin_snapshot.health, Some(PluginBackendHealth::Healthy));
    assert!(!builtin_snapshot.remote_negotiated);

    let external_snapshot = reload
        .runtime_handle_snapshot("repo-inspector")
        .expect("external snapshot should exist");
    assert_eq!(external_snapshot.backend_kind, PluginBackendKind::Process);
    assert_eq!(
        external_snapshot.local_protocol_version.as_deref(),
        Some("5")
    );
    assert_eq!(external_snapshot.health, Some(PluginBackendHealth::Healthy));

    let snapshots = reload.runtime_handle_snapshots();
    assert_eq!(snapshots.len(), 2);

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reload_exposes_unified_capability_bindings() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![builtin, external])
        .await
        .expect("mixed reload should succeed");

    let builtin_binding = reload
        .capability_binding("tool.read")
        .expect("builtin binding should exist");
    assert_eq!(builtin_binding.plugin_id, "core-tools");
    assert_eq!(builtin_binding.backend_kind, PluginBackendKind::InProcess);
    assert_eq!(
        builtin_binding
            .runtime_handle
            .expect("runtime handle should exist")
            .backend_kind,
        PluginBackendKind::InProcess
    );

    let external_binding = reload
        .capability_binding("tool.search")
        .expect("external binding should exist");
    assert_eq!(external_binding.plugin_id, "repo-inspector");
    assert_eq!(external_binding.backend_kind, PluginBackendKind::Process);
    assert_eq!(
        external_binding
            .runtime_handle
            .expect("runtime handle should exist")
            .backend_kind,
        PluginBackendKind::Process
    );

    let bindings = reload.capability_bindings();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].capability.name.to_string(), "tool.read");
    assert_eq!(bindings[1].capability.name.to_string(), "tool.search");

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn reload_prepares_unified_capability_invocation_plan() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .invocation_mode(InvocationMode::Streaming)
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![builtin, external])
        .await
        .expect("mixed reload should succeed");

    let builtin_plan = reload
        .prepare_capability_invocation(
            "tool.read",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
        )
        .expect("builtin invocation plan should exist");
    assert_eq!(builtin_plan.binding.plugin_id, "core-tools");
    assert_eq!(
        builtin_plan.binding.backend_kind,
        PluginBackendKind::InProcess
    );
    assert!(!builtin_plan.stream);
    assert_eq!(builtin_plan.invoke_message.capability, "tool.read");
    assert_eq!(builtin_plan.invoke_message.context.request_id, "req-1");

    let external_plan = reload
        .prepare_capability_invocation(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
        )
        .expect("external invocation plan should exist");
    assert_eq!(external_plan.binding.plugin_id, "repo-inspector");
    assert_eq!(
        external_plan.binding.backend_kind,
        PluginBackendKind::Process
    );
    assert!(external_plan.stream);
    assert_eq!(external_plan.invoke_message.capability, "tool.search");
    assert_eq!(
        external_plan.invoke_message.context.session_id.as_deref(),
        Some("session-1")
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn reload_resolves_unified_capability_invocation_targets() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .invocation_mode(InvocationMode::Streaming)
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![builtin, external])
        .await
        .expect("mixed reload should succeed");

    let builtin_target = reload
        .resolve_capability_invocation_target(
            "tool.read",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
        )
        .expect("builtin target should exist");
    assert_eq!(
        builtin_target.dispatch_kind,
        PluginCapabilityDispatchKind::BuiltinInProcess
    );
    assert_eq!(builtin_target.plan.binding.plugin_id, "core-tools");
    assert!(!builtin_target.plan.stream);

    let external_target = reload
        .resolve_capability_invocation_target(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
        )
        .expect("external target should exist");
    assert_eq!(
        external_target.dispatch_kind,
        PluginCapabilityDispatchKind::ExternalProtocol
    );
    assert_eq!(external_target.plan.binding.plugin_id, "repo-inspector");
    assert!(external_target.plan.stream);

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn reload_reports_capability_dispatch_readiness() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![builtin, external])
        .await
        .expect("mixed reload should succeed");

    assert_eq!(
        reload
            .capability_dispatch_readiness("tool.read")
            .expect("builtin readiness should exist"),
        PluginCapabilityDispatchReadiness::Ready
    );
    assert_eq!(
        reload
            .capability_dispatch_readiness("tool.search")
            .expect("external readiness should exist"),
        PluginCapabilityDispatchReadiness::Ready
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    reload
        .refresh_external_backend_health(&host)
        .expect("health refresh should succeed");
    match reload
        .capability_dispatch_readiness("tool.search")
        .expect("external readiness should exist after shutdown")
    {
        PluginCapabilityDispatchReadiness::BackendUnavailable { message } => {
            assert!(
                message
                    .as_deref()
                    .is_some_and(|value| value.contains("plugin backend exited")),
                "unexpected backend unavailable message: {message:?}"
            );
        },
        other => panic!("unexpected readiness after shutdown: {other:?}"),
    }
}

#[tokio::test]
async fn reload_prepares_ready_capability_dispatch_ticket() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .invocation_mode(InvocationMode::Streaming)
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![builtin, external])
        .await
        .expect("mixed reload should succeed");

    let builtin_ticket = reload
        .prepare_ready_capability_dispatch(
            "tool.read",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
        )
        .expect("builtin dispatch ticket should be ready");
    assert_eq!(
        builtin_ticket.target.dispatch_kind,
        PluginCapabilityDispatchKind::BuiltinInProcess
    );
    assert_eq!(builtin_ticket.target.plan.binding.plugin_id, "core-tools");

    let external_ticket = reload
        .prepare_ready_capability_dispatch(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
        )
        .expect("external dispatch ticket should be ready");
    assert_eq!(
        external_ticket.target.dispatch_kind,
        PluginCapabilityDispatchKind::ExternalProtocol
    );
    assert!(external_ticket.target.plan.stream);

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
    reload
        .refresh_external_backend_health(&host)
        .expect("health refresh should succeed");
    let error = reload
        .prepare_ready_capability_dispatch(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
        )
        .expect_err("unavailable backend should block dispatch ticket creation");
    assert!(error.to_string().contains("插件后端不可用"));
}

#[tokio::test]
async fn reload_dispatches_builtin_with_in_process_executor() {
    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );

    let reload = host
        .reload_from_descriptors(vec![builtin])
        .await
        .expect("builtin reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.read",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
            |plan| {
                assert_eq!(plan.binding.plugin_id, "core-tools");
                assert_eq!(plan.invoke_message.capability, "tool.read");
                Ok(CapabilityExecutionResult::ok(
                    plan.binding.capability.name.to_string(),
                    serde_json::json!({ "content": "hello" }),
                ))
            },
        )
        .expect("builtin dispatch should succeed");

    match outcome {
        PluginCapabilityDispatchOutcome::Completed(result) => {
            assert!(result.success);
            assert_eq!(result.capability_name, "tool.read");
            assert_eq!(result.output, serde_json::json!({ "content": "hello" }));
        },
        other => panic!("unexpected builtin dispatch outcome: {other:?}"),
    }
}

#[tokio::test]
async fn reload_dispatches_builtin_with_registered_executor() {
    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );

    let reload = host
        .reload_from_descriptors(vec![builtin])
        .await
        .expect("builtin reload should succeed");
    let mut registry = BuiltinCapabilityExecutorRegistry::new();
    registry.register("tool.read", Arc::new(StaticBuiltinExecutor));

    let outcome = reload
        .dispatch_capability_with_registry(
            "tool.read",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
            &registry,
        )
        .expect("registered builtin dispatch should succeed");

    match outcome {
        PluginCapabilityDispatchOutcome::Completed(result) => {
            assert!(result.success);
            assert_eq!(result.capability_name, "tool.read");
            assert_eq!(
                result.output,
                serde_json::json!({
                    "executedBy": "core-tools",
                    "input": { "path": "README.md" },
                })
            );
        },
        other => panic!("unexpected builtin dispatch outcome: {other:?}"),
    }
}

#[tokio::test]
async fn reload_dispatches_external_as_protocol_request() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .invocation_mode(InvocationMode::Streaming)
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve into protocol request");

    match outcome {
        PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => {
            assert_eq!(dispatch.runtime_handle.plugin_id, "repo-inspector");
            assert_eq!(
                dispatch.runtime_handle.backend_kind,
                PluginBackendKind::Process
            );
            assert_eq!(
                dispatch.target.dispatch_kind,
                PluginCapabilityDispatchKind::ExternalProtocol
            );
            assert!(dispatch.target.plan.stream);
            assert_eq!(
                dispatch.target.plan.invoke_message.capability,
                "tool.search"
            );
        },
        other => panic!("unexpected external dispatch outcome: {other:?}"),
    }

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn protocol_dispatch_maps_success_result_into_execution_result() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve");

    let dispatch = match outcome {
        PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => dispatch,
        other => panic!("unexpected external dispatch outcome: {other:?}"),
    };

    let result = dispatch.into_execution_result(ResultMessage {
        id: "req-1".to_string(),
        kind: Some("tool_result".to_string()),
        success: true,
        output: serde_json::json!({ "matches": 3 }),
        error: None,
        metadata: serde_json::json!({ "source": "protocol" }),
    });
    assert!(result.success);
    assert_eq!(result.capability_name, "tool.search");
    assert_eq!(result.output, serde_json::json!({ "matches": 3 }));
    assert_eq!(
        result.metadata,
        Some(serde_json::json!({ "source": "protocol" }))
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn protocol_dispatch_maps_failure_result_into_execution_result() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve");

    let dispatch = match outcome {
        PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => dispatch,
        other => panic!("unexpected external dispatch outcome: {other:?}"),
    };

    let result = dispatch.into_execution_result(ResultMessage {
        id: "req-1".to_string(),
        kind: None,
        success: false,
        output: serde_json::Value::Null,
        error: Some(ErrorPayload {
            code: "plugin_failed".to_string(),
            message: "search failed".to_string(),
            details: serde_json::json!({ "query": "plugin-host" }),
            retriable: false,
        }),
        metadata: serde_json::json!({ "source": "protocol" }),
    });
    assert!(!result.success);
    assert_eq!(result.capability_name, "tool.search");
    assert_eq!(result.error.as_deref(), Some("search failed"));
    assert_eq!(
        result.metadata,
        Some(serde_json::json!({ "source": "protocol" }))
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn protocol_dispatch_maps_completed_stream_into_execution_result() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .invocation_mode(InvocationMode::Streaming)
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve");

    let dispatch = match outcome {
        PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => dispatch,
        other => panic!("unexpected external dispatch outcome: {other:?}"),
    };

    let result = dispatch
        .finish_stream_execution_result(vec![
            EventMessage {
                id: "req-1".to_string(),
                phase: EventPhase::Started,
                event: "started".to_string(),
                payload: serde_json::Value::Null,
                seq: 0,
                error: None,
            },
            EventMessage {
                id: "req-1".to_string(),
                phase: EventPhase::Delta,
                event: "chunk".to_string(),
                payload: serde_json::json!({ "text": "hello" }),
                seq: 1,
                error: None,
            },
            EventMessage {
                id: "req-1".to_string(),
                phase: EventPhase::Completed,
                event: "done".to_string(),
                payload: serde_json::json!({ "matches": 3 }),
                seq: 2,
                error: None,
            },
        ])
        .expect("stream completion should map to execution result");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.search");
    assert_eq!(result.output, serde_json::json!({ "matches": 3 }));
    assert_eq!(
        result.metadata,
        Some(serde_json::json!({
            "streamEvents": [{
                "event": "chunk",
                "payload": { "text": "hello" },
                "seq": 1
            }]
        }))
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn protocol_dispatch_maps_failed_stream_into_execution_result() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .invocation_mode(InvocationMode::Streaming)
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve");

    let dispatch = match outcome {
        PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => dispatch,
        other => panic!("unexpected external dispatch outcome: {other:?}"),
    };

    let result = dispatch
        .finish_stream_execution_result(vec![
            EventMessage {
                id: "req-1".to_string(),
                phase: EventPhase::Started,
                event: "started".to_string(),
                payload: serde_json::Value::Null,
                seq: 0,
                error: None,
            },
            EventMessage {
                id: "req-1".to_string(),
                phase: EventPhase::Delta,
                event: "chunk".to_string(),
                payload: serde_json::json!({ "text": "hello" }),
                seq: 1,
                error: None,
            },
            EventMessage {
                id: "req-1".to_string(),
                phase: EventPhase::Failed,
                event: "failed".to_string(),
                payload: serde_json::Value::Null,
                seq: 2,
                error: Some(ErrorPayload {
                    code: "stream_failed".to_string(),
                    message: "stream failed".to_string(),
                    details: serde_json::json!({ "reason": "boom" }),
                    retriable: false,
                }),
            },
        ])
        .expect("stream failure should map to execution result");

    assert!(!result.success);
    assert_eq!(result.capability_name, "tool.search");
    assert_eq!(result.error.as_deref(), Some("stream failed"));
    assert_eq!(
        result.metadata,
        Some(serde_json::json!({
            "streamEvents": [{
                "event": "chunk",
                "payload": { "text": "hello" },
                "seq": 1
            }]
        }))
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[test]
fn completed_dispatch_outcome_executes_without_external_dispatchers() {
    let outcome = PluginCapabilityDispatchOutcome::Completed(CapabilityExecutionResult::ok(
        "tool.read",
        serde_json::json!({ "content": "hello" }),
    ));

    let result = outcome
        .execute_with_dispatchers(
            &StaticProtocolDispatcher {
                execution: PluginCapabilityProtocolExecution::Unary(ResultMessage::success(
                    "ignored",
                    serde_json::Value::Null,
                )),
            },
            &StaticHttpDispatcher,
        )
        .expect("completed outcome should pass through");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.read");
    assert_eq!(result.output, serde_json::json!({ "content": "hello" }));
}

#[tokio::test]
async fn protocol_dispatch_outcome_executes_via_protocol_dispatcher() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve");

    let result = outcome
        .execute_with_dispatchers(
            &StaticProtocolDispatcher {
                execution: PluginCapabilityProtocolExecution::Unary(ResultMessage {
                    id: "req-1".to_string(),
                    kind: Some("tool_result".to_string()),
                    success: true,
                    output: serde_json::json!({ "matches": 5 }),
                    error: None,
                    metadata: serde_json::json!({ "source": "protocol-dispatcher" }),
                }),
            },
            &StaticHttpDispatcher,
        )
        .expect("protocol dispatcher should execute outcome");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.search");
    assert_eq!(result.output, serde_json::json!({ "matches": 5 }));

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[test]
fn http_dispatch_outcome_executes_via_http_dispatcher() {
    let binding = PluginCapabilityBinding {
        plugin_id: "http-plugin".to_string(),
        display_name: "HTTP Plugin".to_string(),
        source_ref: "https://plugins.example.com/http-plugin".to_string(),
        backend_kind: PluginBackendKind::Http,
        capability: CapabilityWireDescriptor::builder(
            "tool.fetch",
            astrcode_core::CapabilityKind::tool(),
        )
        .description("fetch data")
        .input_schema(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .output_schema(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .build()
        .expect("http capability should build"),
        runtime_handle: None,
    };
    let target = super::PluginCapabilityInvocationTarget {
        dispatch_kind: PluginCapabilityDispatchKind::ExternalHttp,
        plan: super::PluginCapabilityInvocationPlan {
            binding,
            payload: serde_json::json!({ "url": "https://example.com" }),
            stream: false,
            invocation_context: super::to_plugin_invocation_context(
                &sample_capability_context(),
                "tool.fetch",
            ),
            invoke_message: InvokeMessage {
                id: "req-1".to_string(),
                capability: "tool.fetch".to_string(),
                input: serde_json::json!({ "url": "https://example.com" }),
                context: super::to_plugin_invocation_context(
                    &sample_capability_context(),
                    "tool.fetch",
                ),
                stream: false,
            },
        },
    };
    let outcome =
        PluginCapabilityDispatchOutcome::ExternalHttp(PluginCapabilityHttpDispatch { target });

    let result = outcome
        .execute_with_dispatchers(
            &StaticProtocolDispatcher {
                execution: PluginCapabilityProtocolExecution::Unary(ResultMessage::success(
                    "ignored",
                    serde_json::Value::Null,
                )),
            },
            &StaticHttpDispatcher,
        )
        .expect("http dispatcher should execute outcome");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.fetch");
    assert_eq!(
        result.output,
        serde_json::json!({
            "transport": "http",
            "pluginId": "http-plugin",
        })
    );
}

#[tokio::test]
async fn transport_backed_protocol_dispatcher_uses_unary_transport_for_non_streaming() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve");

    let result = outcome
        .execute_with_dispatchers(
            &TransportBackedProtocolDispatcher::new(FakeProtocolTransport {
                unary: Some(ResultMessage {
                    id: "req-1".to_string(),
                    kind: Some("tool_result".to_string()),
                    success: true,
                    output: serde_json::json!({ "matches": 17 }),
                    error: None,
                    metadata: serde_json::json!({ "source": "transport" }),
                }),
                stream: Vec::new(),
            }),
            &StaticHttpDispatcher,
        )
        .expect("transport-backed protocol dispatcher should execute unary result");

    assert!(result.success);
    assert_eq!(result.output, serde_json::json!({ "matches": 17 }));

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn transport_backed_protocol_dispatcher_uses_stream_transport_for_streaming() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .invocation_mode(InvocationMode::Streaming)
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let outcome = reload
        .dispatch_capability_with_builtin_executor(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            |_| panic!("external capability should not use builtin executor"),
        )
        .expect("external dispatch should resolve");

    let result = outcome
        .execute_with_dispatchers(
            &TransportBackedProtocolDispatcher::new(FakeProtocolTransport {
                unary: None,
                stream: vec![
                    EventMessage {
                        id: "req-1".to_string(),
                        phase: EventPhase::Started,
                        event: "started".to_string(),
                        payload: serde_json::Value::Null,
                        seq: 0,
                        error: None,
                    },
                    EventMessage {
                        id: "req-1".to_string(),
                        phase: EventPhase::Delta,
                        event: "chunk".to_string(),
                        payload: serde_json::json!({ "text": "hello" }),
                        seq: 1,
                        error: None,
                    },
                    EventMessage {
                        id: "req-1".to_string(),
                        phase: EventPhase::Completed,
                        event: "done".to_string(),
                        payload: serde_json::json!({ "matches": 19 }),
                        seq: 2,
                        error: None,
                    },
                ],
            }),
            &StaticHttpDispatcher,
        )
        .expect("transport-backed protocol dispatcher should execute stream result");

    assert!(result.success);
    assert_eq!(result.output, serde_json::json!({ "matches": 19 }));
    assert_eq!(
        result.metadata,
        Some(serde_json::json!({
            "streamEvents": [{
                "event": "chunk",
                "payload": { "text": "hello" },
                "seq": 1
            }]
        }))
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn execute_capability_with_registries_runs_builtin_path() {
    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );

    let reload = host
        .reload_from_descriptors(vec![builtin])
        .await
        .expect("builtin reload should succeed");
    let mut builtin_registry = BuiltinCapabilityExecutorRegistry::new();
    builtin_registry.register("tool.read", Arc::new(StaticBuiltinExecutor));

    let result = reload
        .execute_capability_with_registries(
            "tool.read",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
            &builtin_registry,
            &PluginCapabilityProtocolDispatcherRegistry::new(),
            &PluginCapabilityHttpDispatcherRegistry::new(),
        )
        .expect("builtin execution should succeed");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.read");
    assert_eq!(
        result.output,
        serde_json::json!({
            "executedBy": "core-tools",
            "input": { "path": "README.md" },
        })
    );
}

#[tokio::test]
async fn execute_capability_with_registries_runs_protocol_path() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");
    let mut protocol_registry = PluginCapabilityProtocolDispatcherRegistry::new();
    protocol_registry.register(
        "repo-inspector",
        Arc::new(StaticProtocolDispatcher {
            execution: PluginCapabilityProtocolExecution::Unary(ResultMessage {
                id: "req-1".to_string(),
                kind: Some("tool_result".to_string()),
                success: true,
                output: serde_json::json!({ "matches": 7 }),
                error: None,
                metadata: serde_json::json!({ "source": "registry" }),
            }),
        }),
    );

    let result = reload
        .execute_capability_with_registries(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            &BuiltinCapabilityExecutorRegistry::new(),
            &protocol_registry,
            &PluginCapabilityHttpDispatcherRegistry::new(),
        )
        .expect("protocol execution should succeed");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.search");
    assert_eq!(result.output, serde_json::json!({ "matches": 7 }));

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn execute_capability_with_registries_runs_http_path() {
    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("remote-fetch", "Remote Fetch");
    external.source_kind = PluginSourceKind::Http;
    external.source_ref = "https://plugins.example.com/remote-fetch".to_string();
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.fetch", astrcode_core::CapabilityKind::tool())
            .description("fetch remote data")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("http capability should build"),
    );

    let reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("http reload should succeed");
    let mut http_registry = PluginCapabilityHttpDispatcherRegistry::new();
    http_registry.register("remote-fetch", Arc::new(StaticHttpDispatcher));

    let result = reload
        .execute_capability_with_registries(
            "tool.fetch",
            serde_json::json!({ "url": "https://example.com" }),
            &sample_capability_context(),
            &BuiltinCapabilityExecutorRegistry::new(),
            &PluginCapabilityProtocolDispatcherRegistry::new(),
            &http_registry,
        )
        .expect("http execution should succeed");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.fetch");
    assert_eq!(
        result.output,
        serde_json::json!({
            "transport": "http",
            "pluginId": "remote-fetch",
        })
    );
}

#[tokio::test]
async fn execute_capability_runs_builtin_path_with_dispatcher_set() {
    let host = PluginHost::new();
    let mut builtin = PluginDescriptor::builtin("core-tools", "Core Tools");
    builtin.tools.push(
        CapabilityWireDescriptor::builder("tool.read", astrcode_core::CapabilityKind::tool())
            .description("read file")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("builtin capability should build"),
    );

    let reload = host
        .reload_from_descriptors(vec![builtin])
        .await
        .expect("builtin reload should succeed");
    let mut dispatchers = PluginCapabilityDispatcherSet::new();
    dispatchers.register_builtin("tool.read", Arc::new(StaticBuiltinExecutor));

    let result = reload
        .execute_capability(
            "tool.read",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
            &dispatchers,
        )
        .expect("builtin execution should succeed");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.read");
}

#[tokio::test]
async fn execute_capability_runs_protocol_path_with_dispatcher_set() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");
    let mut dispatchers = PluginCapabilityDispatcherSet::new();
    dispatchers.register_protocol(
        "repo-inspector",
        Arc::new(StaticProtocolDispatcher {
            execution: PluginCapabilityProtocolExecution::Unary(ResultMessage {
                id: "req-1".to_string(),
                kind: Some("tool_result".to_string()),
                success: true,
                output: serde_json::json!({ "matches": 11 }),
                error: None,
                metadata: serde_json::json!({ "source": "dispatcher-set" }),
            }),
        }),
    );

    let result = reload
        .execute_capability(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            &dispatchers,
        )
        .expect("protocol execution should succeed");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.search");
    assert_eq!(result.output, serde_json::json!({ "matches": 11 }));

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn execute_capability_live_uses_real_external_runtime_handle() {
    let (command, args) = node_protocol_command();

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(command);
    external.launch_args = args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.echo", astrcode_core::CapabilityKind::tool())
            .description("echo payload")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("echo capability should build"),
    );
    external.tools.push(
        CapabilityWireDescriptor::builder(
            "tool.patch_stream",
            astrcode_core::CapabilityKind::tool(),
        )
        .description("stream patch result")
        .input_schema(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .output_schema(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .invocation_mode(InvocationMode::Streaming)
        .build()
        .expect("stream capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let unary = reload
        .execute_capability_live(
            "tool.echo",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
            &PluginCapabilityDispatcherSet::new(),
        )
        .await
        .expect("live unary execution should succeed");

    assert!(unary.success);
    assert_eq!(
        unary.output,
        serde_json::json!({ "echoed": { "path": "README.md" } })
    );
    assert!(
        reload
            .runtime_handle_snapshot("repo-inspector")
            .expect("runtime snapshot should exist")
            .remote_negotiated
    );

    let stream = reload
        .execute_capability_live(
            "tool.patch_stream",
            serde_json::json!({ "path": "src/main.rs" }),
            &sample_capability_context(),
            &PluginCapabilityDispatcherSet::new(),
        )
        .await
        .expect("live streaming execution should succeed");

    assert!(stream.success);
    assert_eq!(stream.output, serde_json::json!({ "ok": true }));
    assert_eq!(
        stream.metadata,
        Some(serde_json::json!({
            "streamEvents": [{
                "event": "tool.delta",
                "payload": { "chunk": 1 },
                "seq": 1
            }]
        }))
    );

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn execute_capability_live_rechecks_external_backend_health_before_dispatch() {
    let (command, args) = node_protocol_command();

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(command);
    external.launch_args = args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.echo", astrcode_core::CapabilityKind::tool())
            .description("echo payload")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("echo capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }

    let error = reload
        .execute_capability_live(
            "tool.echo",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
            &PluginCapabilityDispatcherSet::new(),
        )
        .await
        .expect_err("dead external backend should not dispatch");

    assert!(error.to_string().contains("插件后端不可用"));
    let snapshot = reload
        .runtime_handle_snapshot("repo-inspector")
        .expect("runtime snapshot should still exist");
    assert_eq!(snapshot.health, Some(PluginBackendHealth::Unavailable));
}

#[tokio::test]
async fn execute_capability_live_refreshes_backend_health_after_runtime_invoke_failure() {
    let (command, args) = node_protocol_command_exit_after_initialize();

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(command);
    external.launch_args = args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.echo", astrcode_core::CapabilityKind::tool())
            .description("echo payload")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("echo capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");

    let error = reload
        .execute_capability_live(
            "tool.echo",
            serde_json::json!({ "path": "README.md" }),
            &sample_capability_context(),
            &PluginCapabilityDispatcherSet::new(),
        )
        .await
        .expect_err("invoke after remote exit should fail");

    assert!(
        error.to_string().contains("transport closed")
            || error.to_string().contains("failed to read plugin payload")
    );
    let snapshot = reload
        .runtime_handle_snapshot("repo-inspector")
        .expect("runtime snapshot should still exist");
    assert_eq!(snapshot.health, Some(PluginBackendHealth::Unavailable));
}

#[tokio::test]
async fn execute_capability_runs_http_path_with_dispatcher_set() {
    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("remote-fetch", "Remote Fetch");
    external.source_kind = PluginSourceKind::Http;
    external.source_ref = "https://plugins.example.com/remote-fetch".to_string();
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.fetch", astrcode_core::CapabilityKind::tool())
            .description("fetch remote data")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("http capability should build"),
    );

    let reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("http reload should succeed");
    let mut dispatchers = PluginCapabilityDispatcherSet::new();
    dispatchers.register_http("remote-fetch", Arc::new(StaticHttpDispatcher));

    let result = reload
        .execute_capability(
            "tool.fetch",
            serde_json::json!({ "url": "https://example.com" }),
            &sample_capability_context(),
            &dispatchers,
        )
        .expect("http execution should succeed");

    assert!(result.success);
    assert_eq!(result.capability_name, "tool.fetch");
    assert_eq!(
        result.output,
        serde_json::json!({
            "transport": "http",
            "pluginId": "remote-fetch",
        })
    );
}

#[tokio::test]
async fn execute_capability_uses_default_protocol_dispatcher_when_plugin_specific_missing() {
    #[cfg(windows)]
    let process_command = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(windows))]
    let process_command = "/bin/sh".to_string();
    #[cfg(windows)]
    let process_args = vec!["/C".to_string(), "ping 127.0.0.1 -n 5 >nul".to_string()];
    #[cfg(not(windows))]
    let process_args = vec!["-c".to_string(), "sleep 2".to_string()];

    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
    external.source_kind = PluginSourceKind::Process;
    external.source_ref = "plugins/repo-inspector.toml".to_string();
    external.launch_command = Some(process_command);
    external.launch_args = process_args;
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.search", astrcode_core::CapabilityKind::tool())
            .description("search repository")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("external capability should build"),
    );

    let mut reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("external reload should succeed");
    let mut dispatchers = PluginCapabilityDispatcherSet::new();
    dispatchers.register_default_protocol(Arc::new(StaticProtocolDispatcher {
        execution: PluginCapabilityProtocolExecution::Unary(ResultMessage {
            id: "req-1".to_string(),
            kind: Some("tool_result".to_string()),
            success: true,
            output: serde_json::json!({ "matches": 13 }),
            error: None,
            metadata: serde_json::json!({ "source": "default-protocol" }),
        }),
    }));

    let result = reload
        .execute_capability(
            "tool.search",
            serde_json::json!({ "query": "plugin-host" }),
            &sample_capability_context(),
            &dispatchers,
        )
        .expect("default protocol dispatcher should execute capability");

    assert!(result.success);
    assert_eq!(result.output, serde_json::json!({ "matches": 13 }));

    for backend in &mut reload.external_backends {
        backend.shutdown().await.expect("shutdown should succeed");
    }
}

#[tokio::test]
async fn execute_capability_uses_default_http_dispatcher_when_plugin_specific_missing() {
    let host = PluginHost::new();
    let mut external = PluginDescriptor::builtin("remote-fetch", "Remote Fetch");
    external.source_kind = PluginSourceKind::Http;
    external.source_ref = "https://plugins.example.com/remote-fetch".to_string();
    external.tools.push(
        CapabilityWireDescriptor::builder("tool.fetch", astrcode_core::CapabilityKind::tool())
            .description("fetch remote data")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .output_schema(serde_json::json!({
                "type": "object",
                "properties": {}
            }))
            .build()
            .expect("http capability should build"),
    );

    let reload = host
        .reload_from_descriptors(vec![external])
        .await
        .expect("http reload should succeed");
    let mut dispatchers = PluginCapabilityDispatcherSet::new();
    dispatchers.register_default_http(Arc::new(StaticHttpDispatcher));

    let result = reload
        .execute_capability(
            "tool.fetch",
            serde_json::json!({ "url": "https://example.com" }),
            &sample_capability_context(),
            &dispatchers,
        )
        .expect("default http dispatcher should execute capability");

    assert!(result.success);
    assert_eq!(
        result.output,
        serde_json::json!({
            "transport": "http",
            "pluginId": "remote-fetch",
        })
    );
}
