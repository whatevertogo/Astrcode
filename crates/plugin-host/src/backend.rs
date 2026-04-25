use std::{
    process::Stdio,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use astrcode_core::{AstrError, Result};
use astrcode_protocol::plugin::{EventMessage, InitializeResultData, InvokeMessage, ResultMessage};
use tokio::process::{Child, Command};

use crate::{
    PluginDescriptor, PluginInitializeState, RemotePluginHandshakeSummary,
    transport::PluginStdioTransport,
};

/// plugin-host 视角下的后端执行形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginBackendKind {
    InProcess,
    Process,
    Command,
    Http,
}

/// 统一的后端启动计划。
///
/// 第一阶段只抽出描述层，不直接启动进程或建立 RPC。
/// 这样后续把旧 `process/peer/supervisor` 迁进来时，
/// 可以直接消费这份计划，而不用重新解释 descriptor。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBackendPlan {
    pub plugin_id: String,
    pub backend_kind: PluginBackendKind,
    pub source_ref: String,
    pub launch_command: Option<String>,
    pub launch_args: Vec<String>,
    pub working_dir: Option<String>,
}

/// 外部插件子进程状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginProcessStatus {
    pub running: bool,
    pub exit_code: Option<i32>,
}

/// 外部 backend 的最小健康状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginBackendHealth {
    Healthy,
    Unavailable,
}

/// 外部 backend 的最小健康报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginBackendHealthReport {
    pub plugin_id: String,
    pub health: PluginBackendHealth,
    pub started_at_ms: u64,
    pub shutdown_requested: bool,
    pub message: Option<String>,
}

/// `plugin-host` 中的最小 process backend。
///
/// 这一层只负责：
/// - 根据 `PluginBackendPlan` 启动子进程
/// - 非阻塞检查状态
/// - 在需要时关闭子进程
///
/// JSON-RPC transport / peer / supervisor 会在后续阶段继续接入。
#[derive(Debug)]
pub struct PluginProcessBackend {
    pub plan: PluginBackendPlan,
    child: Child,
    stdio_transport: Option<Arc<PluginStdioTransport>>,
}

/// plugin-host 持有的最小 builtin runtime handle。
///
/// builtin backend 不需要外部进程，但宿主仍然需要一个统一运行时对象，
/// 避免 builtin/external 在组合根侧继续分裂成两套消费方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinPluginRuntimeHandle {
    pub plugin_id: String,
    pub started_at_ms: u64,
}

/// plugin-host 持有的最小 external runtime handle。
///
/// 先把宿主真正关心的状态固化在这里：
/// - 哪个 plugin
/// - 何时启动
/// - 是否已经进入 shutdown 流程
/// - 底层进程 backend
///
/// 之后迁入 peer / supervisor 时，这个 handle 继续当 owner 外壳即可。
#[derive(Debug)]
pub struct ExternalPluginRuntimeHandle {
    pub plugin_id: String,
    pub started_at_ms: u64,
    shutdown_requested: bool,
    backend: PluginProcessBackend,
    protocol_state: Option<PluginInitializeState>,
}

impl PluginBackendPlan {
    pub fn from_descriptor(descriptor: &PluginDescriptor) -> Result<Self> {
        let backend_kind = descriptor.source_kind.to_backend_kind();

        match backend_kind {
            PluginBackendKind::InProcess => Ok(Self {
                plugin_id: descriptor.plugin_id.clone(),
                backend_kind,
                source_ref: descriptor.source_ref.clone(),
                launch_command: None,
                launch_args: Vec::new(),
                working_dir: None,
            }),
            PluginBackendKind::Process | PluginBackendKind::Command => {
                let launch_command = descriptor.launch_command.clone().ok_or_else(|| {
                    AstrError::Validation(format!(
                        "plugin '{}' 缺少 launch_command，无法构建外部插件后端计划",
                        descriptor.plugin_id
                    ))
                })?;
                Ok(Self {
                    plugin_id: descriptor.plugin_id.clone(),
                    backend_kind,
                    source_ref: descriptor.source_ref.clone(),
                    launch_command: Some(launch_command),
                    launch_args: descriptor.launch_args.clone(),
                    working_dir: descriptor.working_dir.clone(),
                })
            },
            PluginBackendKind::Http => {
                if descriptor.source_ref.trim().is_empty() {
                    return Err(AstrError::Validation(format!(
                        "plugin '{}' 缺少 source_ref，无法构建 HTTP 插件后端计划",
                        descriptor.plugin_id
                    )));
                }
                Ok(Self {
                    plugin_id: descriptor.plugin_id.clone(),
                    backend_kind,
                    source_ref: descriptor.source_ref.clone(),
                    launch_command: None,
                    launch_args: Vec::new(),
                    working_dir: None,
                })
            },
        }
    }

    pub async fn start_process(&self) -> Result<PluginProcessBackend> {
        match self.backend_kind {
            PluginBackendKind::Process | PluginBackendKind::Command => {
                let executable = self.launch_command.as_ref().ok_or_else(|| {
                    AstrError::Validation(format!(
                        "plugin '{}' 缺少 launch_command，无法启动外部插件后端",
                        self.plugin_id
                    ))
                })?;
                let mut command = Command::new(executable);
                command
                    .args(&self.launch_args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped());
                if let Some(working_dir) = &self.working_dir {
                    command.current_dir(working_dir);
                }
                let child = command.spawn().map_err(|error| {
                    AstrError::io(
                        format!("failed to spawn plugin backend '{}'", self.plugin_id),
                        error,
                    )
                })?;
                Ok(PluginProcessBackend {
                    plan: self.clone(),
                    child,
                    stdio_transport: None,
                })
            },
            PluginBackendKind::InProcess => Err(AstrError::Validation(format!(
                "plugin '{}' 是 builtin backend，不应走 process 启动路径",
                self.plugin_id
            ))),
            PluginBackendKind::Http => Err(AstrError::Validation(format!(
                "plugin '{}' 是 HTTP backend，当前尚未实现进程启动路径",
                self.plugin_id
            ))),
        }
    }
}

impl PluginProcessBackend {
    pub fn ensure_stdio_transport(&mut self) -> Result<Arc<PluginStdioTransport>> {
        if let Some(transport) = &self.stdio_transport {
            return Ok(Arc::clone(transport));
        }
        let stdin = self.child.stdin.take().ok_or_else(|| {
            AstrError::Internal(format!(
                "plugin backend '{}' did not expose stdin",
                self.plan.plugin_id
            ))
        })?;
        let stdout = self.child.stdout.take().ok_or_else(|| {
            AstrError::Internal(format!(
                "plugin backend '{}' did not expose stdout",
                self.plan.plugin_id
            ))
        })?;
        let transport = Arc::new(PluginStdioTransport::from_child(stdin, stdout));
        self.stdio_transport = Some(Arc::clone(&transport));
        Ok(transport)
    }

    pub fn status(&mut self) -> Result<PluginProcessStatus> {
        let exit_status = self
            .child
            .try_wait()
            .map_err(|error| AstrError::io("failed to poll plugin backend process", error))?;
        Ok(match exit_status {
            Some(status) => PluginProcessStatus {
                running: false,
                exit_code: status.code(),
            },
            None => PluginProcessStatus {
                running: true,
                exit_code: None,
            },
        })
    }

    pub fn health_report(&mut self) -> Result<PluginBackendHealthReport> {
        let status = self.status()?;
        if status.running {
            Ok(PluginBackendHealthReport {
                plugin_id: self.plan.plugin_id.clone(),
                health: PluginBackendHealth::Healthy,
                started_at_ms: 0,
                shutdown_requested: false,
                message: None,
            })
        } else {
            Ok(PluginBackendHealthReport {
                plugin_id: self.plan.plugin_id.clone(),
                health: PluginBackendHealth::Unavailable,
                started_at_ms: 0,
                shutdown_requested: false,
                message: Some(match status.exit_code {
                    Some(code) => format!("plugin backend exited with code {code}"),
                    None => "plugin backend exited".to_string(),
                }),
            })
        }
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        match self.child.kill().await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(AstrError::io(
                format!(
                    "failed to terminate plugin backend '{}'",
                    self.plan.plugin_id
                ),
                error,
            )),
        }
    }
}

impl BuiltinPluginRuntimeHandle {
    pub fn new(plugin_id: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            started_at_ms: now_millis(),
        }
    }

    pub fn health_report(&self) -> PluginBackendHealthReport {
        PluginBackendHealthReport {
            plugin_id: self.plugin_id.clone(),
            health: PluginBackendHealth::Healthy,
            started_at_ms: self.started_at_ms,
            shutdown_requested: false,
            message: None,
        }
    }
}

impl ExternalPluginRuntimeHandle {
    pub fn from_backend(backend: PluginProcessBackend) -> Self {
        Self {
            plugin_id: backend.plan.plugin_id.clone(),
            started_at_ms: now_millis(),
            shutdown_requested: false,
            backend,
            protocol_state: None,
        }
    }

    pub fn with_initialize_state(mut self, state: PluginInitializeState) -> Self {
        self.protocol_state = Some(state);
        self
    }

    pub fn protocol_state(&self) -> Option<&PluginInitializeState> {
        self.protocol_state.as_ref()
    }

    pub fn remote_handshake_summary(&self) -> Option<RemotePluginHandshakeSummary> {
        self.protocol_state
            .as_ref()
            .and_then(PluginInitializeState::remote_handshake_summary)
    }

    pub fn record_remote_initialize(
        &mut self,
        remote_initialize: InitializeResultData,
    ) -> Result<&InitializeResultData> {
        let state = self.protocol_state.as_mut().ok_or_else(|| {
            AstrError::Validation(format!(
                "plugin '{}' 尚未挂接 initialize state，无法记录远端握手结果",
                self.plugin_id
            ))
        })?;
        Ok(state.record_remote_initialize(remote_initialize))
    }

    pub fn backend_kind(&self) -> PluginBackendKind {
        self.backend.plan.backend_kind
    }

    pub fn clone_for_snapshot(&mut self) -> SnapshotExternalPluginRuntimeHandle<'_> {
        SnapshotExternalPluginRuntimeHandle { inner: self }
    }

    pub fn protocol_transport(&mut self) -> Result<Arc<PluginStdioTransport>> {
        self.backend.ensure_stdio_transport()
    }

    pub async fn initialize_remote(&mut self) -> Result<&InitializeResultData> {
        let request = self
            .protocol_state
            .as_ref()
            .ok_or_else(|| {
                AstrError::Validation(format!(
                    "plugin '{}' 尚未挂接 initialize state，无法发起握手",
                    self.plugin_id
                ))
            })?
            .local_initialize
            .clone();
        let transport = self.protocol_transport()?;
        let remote = transport.initialize(&request).await?;
        self.record_remote_initialize(remote)
    }

    pub async fn invoke_unary(&mut self, request: &InvokeMessage) -> Result<ResultMessage> {
        let transport = self.protocol_transport()?;
        transport.invoke_unary(request).await
    }

    /// 向外部插件发送 hook dispatch 请求并等待结果。
    pub async fn dispatch_hook(
        &mut self,
        request: &astrcode_protocol::plugin::HookDispatchMessage,
    ) -> Result<astrcode_protocol::plugin::HookResultMessage> {
        let transport = self.protocol_transport()?;
        transport.dispatch_hook(request).await
    }

    pub async fn invoke_stream(&mut self, request: &InvokeMessage) -> Result<Vec<EventMessage>> {
        let transport = self.protocol_transport()?;
        transport.invoke_stream(request).await
    }

    pub fn status(&mut self) -> Result<PluginProcessStatus> {
        self.backend.status()
    }

    pub fn health_report(&mut self) -> Result<PluginBackendHealthReport> {
        let mut report = self.backend.health_report()?;
        report.started_at_ms = self.started_at_ms;
        report.shutdown_requested = self.shutdown_requested;
        Ok(report)
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.shutdown_requested = true;
        self.backend.shutdown().await
    }
}

pub struct SnapshotExternalPluginRuntimeHandle<'a> {
    inner: &'a mut ExternalPluginRuntimeHandle,
}

impl SnapshotExternalPluginRuntimeHandle<'_> {
    pub fn health_report(&mut self) -> Result<PluginBackendHealthReport> {
        self.inner.health_report()
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use astrcode_protocol::plugin::{
        EventPhase, InitializeResultData, InvokeMessage, PeerDescriptor, PeerRole,
    };
    use serde_json::json;

    use super::{
        BuiltinPluginRuntimeHandle, ExternalPluginRuntimeHandle, PluginBackendHealth,
        PluginBackendKind, PluginBackendPlan,
    };
    use crate::{
        PluginDescriptor, PluginInitializeState, PluginSourceKind, default_initialize_message,
        default_profiles,
    };

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

    #[test]
    fn builtin_descriptor_maps_to_in_process_backend() {
        let descriptor = PluginDescriptor::builtin("core-tools", "Core Tools");

        let plan = PluginBackendPlan::from_descriptor(&descriptor)
            .expect("builtin descriptor should map to backend plan");

        assert_eq!(plan.backend_kind, PluginBackendKind::InProcess);
        assert!(plan.launch_command.is_none());
    }

    #[test]
    fn process_descriptor_requires_launch_command() {
        let mut descriptor = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
        descriptor.source_kind = PluginSourceKind::Process;
        descriptor.source_ref = "plugins/repo-inspector.toml".to_string();

        let error = PluginBackendPlan::from_descriptor(&descriptor)
            .expect_err("process descriptor without command should fail");

        assert!(error.to_string().contains("缺少 launch_command"));
    }

    #[test]
    fn process_descriptor_keeps_launch_fields() {
        let mut descriptor = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
        descriptor.source_kind = PluginSourceKind::Process;
        descriptor.source_ref = "plugins/repo-inspector.toml".to_string();
        descriptor.launch_command = Some("/bin/repo-inspector".to_string());
        descriptor.launch_args = vec!["--stdio".to_string()];
        descriptor.working_dir = Some("/repo".to_string());

        let plan = PluginBackendPlan::from_descriptor(&descriptor)
            .expect("process descriptor should map to backend plan");

        assert_eq!(plan.backend_kind, PluginBackendKind::Process);
        assert_eq!(plan.launch_command.as_deref(), Some("/bin/repo-inspector"));
        assert_eq!(plan.launch_args, vec!["--stdio".to_string()]);
        assert_eq!(plan.working_dir.as_deref(), Some("/repo"));
    }

    #[test]
    fn builtin_runtime_handle_reports_healthy_status() {
        let handle = BuiltinPluginRuntimeHandle::new("core-tools");
        let report = handle.health_report();

        assert_eq!(report.plugin_id, "core-tools");
        assert_eq!(report.health, PluginBackendHealth::Healthy);
        assert!(report.started_at_ms > 0);
        assert!(!report.shutdown_requested);
        assert!(report.message.is_none());
    }

    #[tokio::test]
    async fn process_backend_can_start_check_status_and_shutdown() {
        let (command, args) = shell_command_with_args("ping 127.0.0.1 -n 5 >nul");
        let mut descriptor = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
        descriptor.source_kind = PluginSourceKind::Process;
        descriptor.source_ref = "plugins/repo-inspector.toml".to_string();
        descriptor.launch_command = Some(command);
        descriptor.launch_args = args;

        let plan = PluginBackendPlan::from_descriptor(&descriptor)
            .expect("process descriptor should map to backend plan");
        let mut backend = plan
            .start_process()
            .await
            .expect("process backend should start");

        let status = backend.status().expect("status should be readable");
        assert!(status.running);
        let report = backend
            .health_report()
            .expect("health report should be readable");
        assert_eq!(report.health, PluginBackendHealth::Healthy);

        backend.shutdown().await.expect("shutdown should succeed");
    }

    #[tokio::test]
    async fn external_runtime_handle_tracks_start_and_shutdown_state() {
        let (command, args) = shell_command_with_args("ping 127.0.0.1 -n 5 >nul");
        let mut descriptor = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
        descriptor.source_kind = PluginSourceKind::Process;
        descriptor.source_ref = "plugins/repo-inspector.toml".to_string();
        descriptor.launch_command = Some(command);
        descriptor.launch_args = args;

        let plan = PluginBackendPlan::from_descriptor(&descriptor)
            .expect("process descriptor should map to backend plan");
        let backend = plan
            .start_process()
            .await
            .expect("process backend should start");
        let mut handle = ExternalPluginRuntimeHandle::from_backend(backend);

        let report = handle
            .health_report()
            .expect("health report should be readable");
        assert_eq!(report.plugin_id, "repo-inspector");
        assert_eq!(report.health, PluginBackendHealth::Healthy);
        assert!(report.started_at_ms > 0);
        assert!(!report.shutdown_requested);

        handle.shutdown().await.expect("shutdown should succeed");
    }

    #[tokio::test]
    async fn external_runtime_handle_can_store_initialize_state() {
        let (command, args) = shell_command_with_args("ping 127.0.0.1 -n 5 >nul");
        let mut descriptor = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
        descriptor.source_kind = PluginSourceKind::Process;
        descriptor.source_ref = "plugins/repo-inspector.toml".to_string();
        descriptor.launch_command = Some(command);
        descriptor.launch_args = args;

        let plan = PluginBackendPlan::from_descriptor(&descriptor)
            .expect("process descriptor should map to backend plan");
        let backend = plan
            .start_process()
            .await
            .expect("process backend should start");
        let local_peer = PeerDescriptor {
            id: "host-1".to_string(),
            name: "plugin-host".to_string(),
            role: PeerRole::Supervisor,
            version: "0.1.0".to_string(),
            supported_profiles: vec!["coding".to_string()],
            metadata: serde_json::Value::Null,
        };
        let initialize_state = PluginInitializeState::new(default_initialize_message(
            local_peer.clone(),
            Vec::new(),
            default_profiles(),
        ));
        let mut handle = ExternalPluginRuntimeHandle::from_backend(backend)
            .with_initialize_state(initialize_state);

        assert!(handle.protocol_state().is_some());
        assert_eq!(
            handle
                .protocol_state()
                .expect("protocol state should exist")
                .negotiated_protocol_version(),
            "5"
        );

        handle
            .record_remote_initialize(InitializeResultData {
                protocol_version: "5".to_string(),
                peer: local_peer,
                capabilities: Vec::new(),
                handlers: Vec::new(),
                profiles: default_profiles(),
                skills: Vec::new(),
                modes: Vec::new(),
                metadata: serde_json::Value::Null,
            })
            .expect("record remote initialize should succeed");

        assert!(
            handle
                .protocol_state()
                .expect("protocol state should exist")
                .remote_initialize
                .is_some()
        );
        let summary = handle
            .remote_handshake_summary()
            .expect("remote handshake summary should exist");
        assert_eq!(summary.peer_id, "host-1");
        assert_eq!(summary.profile_names, vec!["coding".to_string()]);

        handle.shutdown().await.expect("shutdown should succeed");
    }

    #[tokio::test]
    async fn builtin_backend_is_rejected_by_process_launcher() {
        let descriptor = PluginDescriptor::builtin("core-tools", "Core Tools");
        let plan = PluginBackendPlan::from_descriptor(&descriptor)
            .expect("builtin descriptor should map to backend plan");

        let error = plan
            .start_process()
            .await
            .expect_err("builtin backend should not start as process");

        assert!(error.to_string().contains("builtin backend"));
    }

    #[tokio::test]
    async fn external_runtime_handle_can_initialize_and_invoke_over_stdio_transport() {
        let (command, args) = node_protocol_command();
        let mut descriptor = PluginDescriptor::builtin("repo-inspector", "Repo Inspector");
        descriptor.source_kind = PluginSourceKind::Process;
        descriptor.source_ref = "plugins/repo-inspector.toml".to_string();
        descriptor.launch_command = Some(command);
        descriptor.launch_args = args;

        let plan = PluginBackendPlan::from_descriptor(&descriptor)
            .expect("process descriptor should map to backend plan");
        let backend = plan
            .start_process()
            .await
            .expect("process backend should start");
        let local_peer = PeerDescriptor {
            id: "host-1".to_string(),
            name: "plugin-host".to_string(),
            role: PeerRole::Supervisor,
            version: "0.1.0".to_string(),
            supported_profiles: vec!["coding".to_string()],
            metadata: serde_json::Value::Null,
        };
        let initialize_state = PluginInitializeState::new(default_initialize_message(
            local_peer,
            Vec::new(),
            default_profiles(),
        ));
        let mut handle = ExternalPluginRuntimeHandle::from_backend(backend)
            .with_initialize_state(initialize_state);

        let negotiated = handle
            .initialize_remote()
            .await
            .expect("initialize should succeed");
        assert_eq!(negotiated.peer.id, "fixture-worker");

        let unary = handle
            .invoke_unary(&InvokeMessage {
                id: "req-1".to_string(),
                capability: "tool.echo".to_string(),
                input: json!({ "path": "README.md" }),
                context: astrcode_protocol::plugin::InvocationContext {
                    request_id: "req-1".to_string(),
                    trace_id: None,
                    session_id: Some("session-1".to_string()),
                    caller: Some(astrcode_protocol::plugin::CallerRef {
                        id: "test".to_string(),
                        role: "integration-test".to_string(),
                        metadata: serde_json::Value::Null,
                    }),
                    workspace: None,
                    deadline_ms: None,
                    budget: None,
                    profile: "coding".to_string(),
                    profile_context: serde_json::Value::Null,
                    metadata: serde_json::Value::Null,
                },
                stream: false,
            })
            .await
            .expect("unary invoke should succeed");
        assert!(unary.success);
        assert_eq!(unary.output, json!({ "echoed": { "path": "README.md" } }));

        let stream = handle
            .invoke_stream(&InvokeMessage {
                id: "req-2".to_string(),
                capability: "tool.patch_stream".to_string(),
                input: json!({ "path": "src/main.rs" }),
                context: astrcode_protocol::plugin::InvocationContext {
                    request_id: "req-2".to_string(),
                    trace_id: None,
                    session_id: Some("session-1".to_string()),
                    caller: Some(astrcode_protocol::plugin::CallerRef {
                        id: "test".to_string(),
                        role: "integration-test".to_string(),
                        metadata: serde_json::Value::Null,
                    }),
                    workspace: None,
                    deadline_ms: None,
                    budget: None,
                    profile: "coding".to_string(),
                    profile_context: serde_json::Value::Null,
                    metadata: serde_json::Value::Null,
                },
                stream: true,
            })
            .await
            .expect("stream invoke should succeed");
        assert_eq!(stream.len(), 3);
        assert_eq!(stream[1].phase, EventPhase::Delta);
        assert_eq!(stream[2].phase, EventPhase::Completed);

        handle.shutdown().await.expect("shutdown should succeed");
    }
}
