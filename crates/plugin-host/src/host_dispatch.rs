use std::{collections::BTreeMap, sync::Arc};

use astrcode_core::{AstrError, CapabilityContext, CapabilityExecutionResult, Result};
use astrcode_protocol::plugin::{
    CapabilityWireDescriptor, EventMessage, EventPhase, InvocationContext, InvokeMessage,
    ResultMessage,
};
use serde_json::Value;

use crate::backend::{BuiltinPluginRuntimeHandle, ExternalPluginRuntimeHandle, PluginBackendKind};

#[derive(Debug, Clone, Copy)]
pub enum PluginRuntimeHandleRef<'a> {
    Builtin(&'a BuiltinPluginRuntimeHandle),
    External(&'a ExternalPluginRuntimeHandle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRuntimeHandleSnapshot {
    pub plugin_id: String,
    pub backend_kind: PluginBackendKind,
    pub started_at_ms: u64,
    pub shutdown_requested: bool,
    pub health: Option<crate::backend::PluginBackendHealth>,
    pub message: Option<String>,
    pub local_protocol_version: Option<String>,
    pub remote_negotiated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginCapabilityBinding {
    pub plugin_id: String,
    pub display_name: String,
    pub backend_kind: PluginBackendKind,
    pub capability: CapabilityWireDescriptor,
    pub runtime_handle: Option<PluginRuntimeHandleSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginCapabilityInvocationPlan {
    pub binding: PluginCapabilityBinding,
    pub payload: Value,
    pub stream: bool,
    pub invocation_context: InvocationContext,
    pub invoke_message: InvokeMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCapabilityDispatchKind {
    BuiltinInProcess,
    ExternalProtocol,
    ExternalHttp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginCapabilityInvocationTarget {
    pub dispatch_kind: PluginCapabilityDispatchKind,
    pub plan: PluginCapabilityInvocationPlan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginCapabilityDispatchTicket {
    pub target: PluginCapabilityInvocationTarget,
}

/// 统一调用 owner 给出的最小分派结果。
///
/// builtin 直接在宿主内执行并返回结果；
/// external backend 先收口成稳定的 dispatch 请求对象，
/// 后续再由真正的 transport / protocol owner 接手。
#[derive(Debug, Clone, PartialEq)]
pub enum PluginCapabilityDispatchOutcome {
    Completed(CapabilityExecutionResult),
    ExternalProtocol(PluginCapabilityProtocolDispatch),
    ExternalHttp(PluginCapabilityHttpDispatch),
}

/// external protocol backend 的最小分派请求。
#[derive(Debug, Clone, PartialEq)]
pub struct PluginCapabilityProtocolDispatch {
    pub runtime_handle: PluginRuntimeHandleSnapshot,
    pub target: PluginCapabilityInvocationTarget,
}

/// external HTTP backend 的最小分派请求。
#[derive(Debug, Clone, PartialEq)]
pub struct PluginCapabilityHttpDispatch {
    pub target: PluginCapabilityInvocationTarget,
}

/// external protocol backend 的最小执行结果。
#[derive(Debug, Clone, PartialEq)]
pub enum PluginCapabilityProtocolExecution {
    Unary(ResultMessage),
    Stream(Vec<EventMessage>),
}

/// external protocol dispatcher 合同。
///
/// transport / peer / supervisor 的真实实现后续可以挂在这层后面，
/// 但 `plugin-host` 先把统一调用 owner 的执行入口固定下来。
pub trait PluginCapabilityProtocolDispatcher {
    fn dispatch(
        &self,
        dispatch: &PluginCapabilityProtocolDispatch,
    ) -> Result<PluginCapabilityProtocolExecution>;
}

/// protocol transport 合同。
///
/// 真实 stdio / rpc / remote peer 实现后续只需要满足这层发送接口，
/// 不需要重新解释 invocation target 或结果映射。
pub trait PluginCapabilityProtocolTransport: Send + Sync {
    fn invoke_unary(&self, dispatch: &PluginCapabilityProtocolDispatch) -> Result<ResultMessage>;

    fn invoke_stream(
        &self,
        dispatch: &PluginCapabilityProtocolDispatch,
    ) -> Result<Vec<EventMessage>>;
}

/// external HTTP dispatcher 合同。
pub trait PluginCapabilityHttpDispatcher {
    fn dispatch(
        &self,
        dispatch: &PluginCapabilityHttpDispatch,
    ) -> Result<CapabilityExecutionResult>;
}

/// protocol dispatcher 注册表。
#[derive(Default)]
pub struct PluginCapabilityProtocolDispatcherRegistry {
    dispatchers: BTreeMap<String, Arc<dyn PluginCapabilityProtocolDispatcher>>,
}

/// HTTP dispatcher 注册表。
#[derive(Default)]
pub struct PluginCapabilityHttpDispatcherRegistry {
    dispatchers: BTreeMap<String, Arc<dyn PluginCapabilityHttpDispatcher>>,
}

/// `plugin-host` 的统一 dispatcher set。
///
/// 组合根后续只需要持有这一份宿主执行集合，
/// 不必在每次调用时再拆开传三份 registry。
#[derive(Default)]
pub struct PluginCapabilityDispatcherSet {
    pub builtin: BuiltinCapabilityExecutorRegistry,
    pub protocol: PluginCapabilityProtocolDispatcherRegistry,
    pub http: PluginCapabilityHttpDispatcherRegistry,
    default_protocol: Option<Arc<dyn PluginCapabilityProtocolDispatcher>>,
    default_http: Option<Arc<dyn PluginCapabilityHttpDispatcher>>,
}

/// 基于 transport 的 protocol dispatcher。
pub struct TransportBackedProtocolDispatcher<T> {
    transport: T,
}

impl PluginCapabilityProtocolDispatch {
    pub fn into_execution_result(self, result: ResultMessage) -> CapabilityExecutionResult {
        let error = if result.success {
            None
        } else {
            Some(
                result
                    .error
                    .map(|value| value.message)
                    .unwrap_or_else(|| "plugin invocation failed".to_string()),
            )
        };
        CapabilityExecutionResult::from_common(
            self.target.plan.binding.capability.name.to_string(),
            result.success,
            result.output,
            None,
            astrcode_core::ExecutionResultCommon {
                error: error.clone(),
                metadata: Some(result.metadata),
                duration_ms: 0,
                truncated: false,
            },
        )
    }

    pub fn finish_stream_execution_result<I>(self, events: I) -> Result<CapabilityExecutionResult>
    where
        I: IntoIterator<Item = EventMessage>,
    {
        let mut deltas = Vec::new();

        for event in events {
            match event.phase {
                EventPhase::Started => {},
                EventPhase::Delta => {
                    deltas.push(serde_json::json!({
                        "event": event.event,
                        "payload": event.payload,
                        "seq": event.seq,
                    }));
                },
                EventPhase::Completed => {
                    return Ok(CapabilityExecutionResult::from_common(
                        self.target.plan.binding.capability.name.to_string(),
                        true,
                        event.payload,
                        None,
                        astrcode_core::ExecutionResultCommon::success(
                            Some(serde_json::json!({ "streamEvents": deltas })),
                            0,
                            false,
                        ),
                    ));
                },
                EventPhase::Failed => {
                    let error = event
                        .error
                        .map(|value| value.message)
                        .unwrap_or_else(|| "stream invocation failed".to_string());
                    return Ok(CapabilityExecutionResult::from_common(
                        self.target.plan.binding.capability.name.to_string(),
                        false,
                        Value::Null,
                        None,
                        astrcode_core::ExecutionResultCommon::failure(
                            error,
                            Some(serde_json::json!({ "streamEvents": deltas })),
                            0,
                            false,
                        ),
                    ));
                },
            }
        }

        Err(AstrError::Internal(
            "plugin stream ended without terminal event".to_string(),
        ))
    }

    pub fn into_execution_result_from_dispatch(
        self,
        execution: PluginCapabilityProtocolExecution,
    ) -> Result<CapabilityExecutionResult> {
        match execution {
            PluginCapabilityProtocolExecution::Unary(result) => {
                Ok(self.into_execution_result(result))
            },
            PluginCapabilityProtocolExecution::Stream(events) => {
                self.finish_stream_execution_result(events)
            },
        }
    }
}

impl<T> TransportBackedProtocolDispatcher<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> PluginCapabilityProtocolDispatcher for TransportBackedProtocolDispatcher<T>
where
    T: PluginCapabilityProtocolTransport,
{
    fn dispatch(
        &self,
        dispatch: &PluginCapabilityProtocolDispatch,
    ) -> Result<PluginCapabilityProtocolExecution> {
        if dispatch.target.plan.stream {
            self.transport
                .invoke_stream(dispatch)
                .map(PluginCapabilityProtocolExecution::Stream)
        } else {
            self.transport
                .invoke_unary(dispatch)
                .map(PluginCapabilityProtocolExecution::Unary)
        }
    }
}

impl PluginCapabilityDispatchOutcome {
    pub fn execute_with_dispatchers<P, H>(
        self,
        protocol_dispatcher: &P,
        http_dispatcher: &H,
    ) -> Result<CapabilityExecutionResult>
    where
        P: PluginCapabilityProtocolDispatcher,
        H: PluginCapabilityHttpDispatcher,
    {
        match self {
            PluginCapabilityDispatchOutcome::Completed(result) => Ok(result),
            PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => dispatch
                .clone()
                .into_execution_result_from_dispatch(protocol_dispatcher.dispatch(&dispatch)?),
            PluginCapabilityDispatchOutcome::ExternalHttp(dispatch) => {
                http_dispatcher.dispatch(&dispatch)
            },
        }
    }
}

/// builtin capability 的最小进程内执行合同。
pub trait BuiltinCapabilityExecutor: Send + Sync {
    fn execute(
        &self,
        plan: &PluginCapabilityInvocationPlan,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult>;
}

/// builtin capability 执行器注册表。
///
/// 这一层先只按 capability name 做最小注册，
/// 让 `plugin-host` 可以真正持有 builtin 的进程内执行入口。
#[derive(Default)]
pub struct BuiltinCapabilityExecutorRegistry {
    executors: BTreeMap<String, Arc<dyn BuiltinCapabilityExecutor>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCapabilityDispatchReadiness {
    Ready,
    MissingRuntimeHandle,
    BackendUnavailable { message: Option<String> },
    ProtocolNotReady,
}

impl BuiltinCapabilityExecutorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        capability_name: impl Into<String>,
        executor: Arc<dyn BuiltinCapabilityExecutor>,
    ) -> Option<Arc<dyn BuiltinCapabilityExecutor>> {
        self.executors.insert(capability_name.into(), executor)
    }

    pub fn executor(&self, capability_name: &str) -> Option<Arc<dyn BuiltinCapabilityExecutor>> {
        self.executors.get(capability_name).cloned()
    }
}

impl PluginCapabilityProtocolDispatcherRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        plugin_id: impl Into<String>,
        dispatcher: Arc<dyn PluginCapabilityProtocolDispatcher>,
    ) -> Option<Arc<dyn PluginCapabilityProtocolDispatcher>> {
        self.dispatchers.insert(plugin_id.into(), dispatcher)
    }

    pub fn dispatcher(
        &self,
        plugin_id: &str,
    ) -> Option<Arc<dyn PluginCapabilityProtocolDispatcher>> {
        self.dispatchers.get(plugin_id).cloned()
    }
}

impl PluginCapabilityHttpDispatcherRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        plugin_id: impl Into<String>,
        dispatcher: Arc<dyn PluginCapabilityHttpDispatcher>,
    ) -> Option<Arc<dyn PluginCapabilityHttpDispatcher>> {
        self.dispatchers.insert(plugin_id.into(), dispatcher)
    }

    pub fn dispatcher(&self, plugin_id: &str) -> Option<Arc<dyn PluginCapabilityHttpDispatcher>> {
        self.dispatchers.get(plugin_id).cloned()
    }
}

impl PluginCapabilityDispatcherSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_builtin(
        &mut self,
        capability_name: impl Into<String>,
        executor: Arc<dyn BuiltinCapabilityExecutor>,
    ) -> Option<Arc<dyn BuiltinCapabilityExecutor>> {
        self.builtin.register(capability_name, executor)
    }

    pub fn register_protocol(
        &mut self,
        plugin_id: impl Into<String>,
        dispatcher: Arc<dyn PluginCapabilityProtocolDispatcher>,
    ) -> Option<Arc<dyn PluginCapabilityProtocolDispatcher>> {
        self.protocol.register(plugin_id, dispatcher)
    }

    pub fn register_default_protocol(
        &mut self,
        dispatcher: Arc<dyn PluginCapabilityProtocolDispatcher>,
    ) -> Option<Arc<dyn PluginCapabilityProtocolDispatcher>> {
        self.default_protocol.replace(dispatcher)
    }

    pub fn register_http(
        &mut self,
        plugin_id: impl Into<String>,
        dispatcher: Arc<dyn PluginCapabilityHttpDispatcher>,
    ) -> Option<Arc<dyn PluginCapabilityHttpDispatcher>> {
        self.http.register(plugin_id, dispatcher)
    }

    pub fn register_default_http(
        &mut self,
        dispatcher: Arc<dyn PluginCapabilityHttpDispatcher>,
    ) -> Option<Arc<dyn PluginCapabilityHttpDispatcher>> {
        self.default_http.replace(dispatcher)
    }

    pub fn protocol_dispatcher_for(
        &self,
        plugin_id: &str,
    ) -> Option<Arc<dyn PluginCapabilityProtocolDispatcher>> {
        self.protocol
            .dispatcher(plugin_id)
            .or_else(|| self.default_protocol.clone())
    }

    pub fn http_dispatcher_for(
        &self,
        plugin_id: &str,
    ) -> Option<Arc<dyn PluginCapabilityHttpDispatcher>> {
        self.http
            .dispatcher(plugin_id)
            .or_else(|| self.default_http.clone())
    }
}
