use astrcode_core::{
    AstrError, CapabilityContext, CapabilityExecutionResult, InvocationMode, Result,
};
use astrcode_protocol::plugin::{CapabilityWireDescriptor, InitializeResultData, InvokeMessage};
use serde_json::Value;

use super::{
    PluginHost, PluginHostReload,
    catalog::{ActivePluginRuntimeCatalog, ExternalBackendHealthCatalog},
    dispatch::{
        BuiltinCapabilityExecutorRegistry, PluginCapabilityBinding, PluginCapabilityDispatchKind,
        PluginCapabilityDispatchOutcome, PluginCapabilityDispatchReadiness,
        PluginCapabilityDispatchTicket, PluginCapabilityDispatcherSet,
        PluginCapabilityHttpDispatch, PluginCapabilityHttpDispatcherRegistry,
        PluginCapabilityInvocationPlan, PluginCapabilityInvocationTarget,
        PluginCapabilityProtocolDispatch, PluginCapabilityProtocolDispatcherRegistry,
        PluginCapabilityProtocolExecution, PluginRuntimeHandleRef, PluginRuntimeHandleSnapshot,
    },
    to_plugin_invocation_context,
};
use crate::{
    PluginDescriptor,
    backend::{
        BuiltinPluginRuntimeHandle, PluginBackendHealth, PluginBackendHealthReport,
        PluginBackendKind,
    },
    descriptor::{
        CommandDescriptor, HookDescriptor, PromptDescriptor, ProviderDescriptor,
        ResourceDescriptor, SkillDescriptor, ThemeDescriptor,
    },
};

/// 在所有 plugin descriptor 中按字段名查找贡献项，返回 (所在 descriptor, 匹配项)。
macro_rules! define_descriptor_lookup {
    ($method:ident, $field:ident, $id_field:ident, $item_type:ty) => {
        pub fn $method(&self, id: &str) -> Option<(&PluginDescriptor, &$item_type)> {
            self.descriptors.iter().find_map(|descriptor| {
                descriptor
                    .$field
                    .iter()
                    .find(|item| item.$id_field.as_str() == id)
                    .map(|item| (descriptor, item))
            })
        }
    };
}

impl PluginHostReload {
    pub async fn execute_capability_live(
        &mut self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
        dispatchers: &PluginCapabilityDispatcherSet,
    ) -> Result<CapabilityExecutionResult> {
        self.refresh_backend_health_from_runtime_handles()?;
        let target = self
            .prepare_ready_capability_dispatch(capability_name, payload, ctx)?
            .target;

        match target.dispatch_kind {
            PluginCapabilityDispatchKind::BuiltinInProcess => {
                let executor = dispatchers
                    .builtin
                    .executor(capability_name)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "能力 '{}' 缺少 builtin executor 注册",
                            capability_name
                        ))
                    })?;
                executor.execute(&target.plan, ctx)
            },
            PluginCapabilityDispatchKind::ExternalProtocol => {
                self.execute_protocol_capability_live(target).await
            },
            PluginCapabilityDispatchKind::ExternalHttp => {
                let dispatcher = dispatchers
                    .http_dispatcher_for(&target.plan.binding.plugin_id)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "plugin '{}' 缺少 http dispatcher 注册",
                            target.plan.binding.plugin_id
                        ))
                    })?;
                dispatcher.dispatch(&PluginCapabilityHttpDispatch { target })
            },
        }
    }

    pub fn execute_capability(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
        dispatchers: &PluginCapabilityDispatcherSet,
    ) -> Result<CapabilityExecutionResult> {
        let binding = self.capability_binding(capability_name).ok_or_else(|| {
            AstrError::Validation(format!("plugin-host 中不存在能力 '{}'", capability_name))
        })?;
        let outcome = self.dispatch_capability_with_registry(
            capability_name,
            payload,
            ctx,
            &dispatchers.builtin,
        )?;
        match outcome {
            PluginCapabilityDispatchOutcome::Completed(result) => Ok(result),
            PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => {
                let dispatcher = dispatchers
                    .protocol_dispatcher_for(&binding.plugin_id)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "plugin '{}' 缺少 protocol dispatcher 注册",
                            binding.plugin_id
                        ))
                    })?;
                dispatch
                    .clone()
                    .into_execution_result_from_dispatch(dispatcher.dispatch(&dispatch)?)
            },
            PluginCapabilityDispatchOutcome::ExternalHttp(dispatch) => {
                let dispatcher = dispatchers
                    .http_dispatcher_for(&binding.plugin_id)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "plugin '{}' 缺少 http dispatcher 注册",
                            binding.plugin_id
                        ))
                    })?;
                dispatcher.dispatch(&dispatch)
            },
        }
    }

    pub fn execute_capability_with_registries(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
        builtin_registry: &BuiltinCapabilityExecutorRegistry,
        protocol_registry: &PluginCapabilityProtocolDispatcherRegistry,
        http_registry: &PluginCapabilityHttpDispatcherRegistry,
    ) -> Result<CapabilityExecutionResult> {
        let binding = self.capability_binding(capability_name).ok_or_else(|| {
            AstrError::Validation(format!("plugin-host 中不存在能力 '{}'", capability_name))
        })?;
        let outcome = self.dispatch_capability_with_registry(
            capability_name,
            payload,
            ctx,
            builtin_registry,
        )?;
        match outcome {
            PluginCapabilityDispatchOutcome::Completed(result) => Ok(result),
            PluginCapabilityDispatchOutcome::ExternalProtocol(dispatch) => {
                let dispatcher = protocol_registry
                    .dispatcher(&binding.plugin_id)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "plugin '{}' 缺少 protocol dispatcher 注册",
                            binding.plugin_id
                        ))
                    })?;
                dispatch
                    .clone()
                    .into_execution_result_from_dispatch(dispatcher.dispatch(&dispatch)?)
            },
            PluginCapabilityDispatchOutcome::ExternalHttp(dispatch) => {
                let dispatcher = http_registry
                    .dispatcher(&binding.plugin_id)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "plugin '{}' 缺少 http dispatcher 注册",
                            binding.plugin_id
                        ))
                    })?;
                dispatcher.dispatch(&dispatch)
            },
        }
    }

    pub fn dispatch_capability_with_registry(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
        registry: &BuiltinCapabilityExecutorRegistry,
    ) -> Result<PluginCapabilityDispatchOutcome> {
        self.dispatch_capability_with_builtin_executor(capability_name, payload, ctx, |plan| {
            let executor = registry.executor(capability_name).ok_or_else(|| {
                AstrError::Validation(format!(
                    "能力 '{}' 缺少 builtin executor 注册",
                    capability_name
                ))
            })?;
            executor.execute(plan, ctx)
        })
    }

    pub fn dispatch_capability_with_builtin_executor<F>(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
        builtin_executor: F,
    ) -> Result<PluginCapabilityDispatchOutcome>
    where
        F: FnOnce(&PluginCapabilityInvocationPlan) -> Result<CapabilityExecutionResult>,
    {
        let ticket = self.prepare_ready_capability_dispatch(capability_name, payload, ctx)?;
        match ticket.target.dispatch_kind {
            PluginCapabilityDispatchKind::BuiltinInProcess => builtin_executor(&ticket.target.plan)
                .map(PluginCapabilityDispatchOutcome::Completed),
            PluginCapabilityDispatchKind::ExternalProtocol => {
                let runtime_handle = ticket
                    .target
                    .plan
                    .binding
                    .runtime_handle
                    .clone()
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "能力 '{}' 的 external backend 缺少运行时快照",
                            capability_name
                        ))
                    })?;
                Ok(PluginCapabilityDispatchOutcome::ExternalProtocol(
                    PluginCapabilityProtocolDispatch {
                        runtime_handle,
                        target: ticket.target,
                    },
                ))
            },
            PluginCapabilityDispatchKind::ExternalHttp => Ok(
                PluginCapabilityDispatchOutcome::ExternalHttp(PluginCapabilityHttpDispatch {
                    target: ticket.target,
                }),
            ),
        }
    }

    pub fn prepare_ready_capability_dispatch(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<PluginCapabilityDispatchTicket> {
        let readiness = self
            .capability_dispatch_readiness(capability_name)
            .ok_or_else(|| {
                AstrError::Validation(format!("plugin-host 中不存在能力 '{}'", capability_name))
            })?;
        match readiness {
            PluginCapabilityDispatchReadiness::Ready => {
                let target = self
                    .resolve_capability_invocation_target(capability_name, payload, ctx)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "plugin-host 无法解析能力 '{}' 的调用目标",
                            capability_name
                        ))
                    })?;
                Ok(PluginCapabilityDispatchTicket { target })
            },
            PluginCapabilityDispatchReadiness::MissingRuntimeHandle => Err(AstrError::Validation(
                format!("能力 '{}' 缺少运行时句柄，无法分派", capability_name),
            )),
            PluginCapabilityDispatchReadiness::BackendUnavailable { message } => {
                Err(AstrError::Validation(format!(
                    "能力 '{}' 的插件后端不可用: {}",
                    capability_name,
                    message.unwrap_or_else(|| "unknown backend state".to_string())
                )))
            },
            PluginCapabilityDispatchReadiness::ProtocolNotReady => Err(AstrError::Validation(
                format!("能力 '{}' 的协议握手尚未完成，无法分派", capability_name),
            )),
        }
    }

    pub fn capability_dispatch_readiness(
        &self,
        capability_name: &str,
    ) -> Option<PluginCapabilityDispatchReadiness> {
        let binding = self.capability_binding(capability_name)?;
        let runtime_handle = binding.runtime_handle.as_ref();

        let readiness = match binding.backend_kind {
            PluginBackendKind::InProcess => {
                if runtime_handle.is_some() {
                    PluginCapabilityDispatchReadiness::Ready
                } else {
                    PluginCapabilityDispatchReadiness::MissingRuntimeHandle
                }
            },
            PluginBackendKind::Process | PluginBackendKind::Command => match runtime_handle {
                None => PluginCapabilityDispatchReadiness::MissingRuntimeHandle,
                Some(handle) => match handle.health.clone() {
                    Some(crate::backend::PluginBackendHealth::Unavailable) => {
                        PluginCapabilityDispatchReadiness::BackendUnavailable {
                            message: handle.message.clone(),
                        }
                    },
                    Some(crate::backend::PluginBackendHealth::Healthy) => {
                        if handle.local_protocol_version.is_some() {
                            PluginCapabilityDispatchReadiness::Ready
                        } else {
                            PluginCapabilityDispatchReadiness::ProtocolNotReady
                        }
                    },
                    None => PluginCapabilityDispatchReadiness::ProtocolNotReady,
                },
            },
            PluginBackendKind::Http => PluginCapabilityDispatchReadiness::Ready,
        };

        Some(readiness)
    }

    pub fn resolve_capability_invocation_target(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Option<PluginCapabilityInvocationTarget> {
        let plan = self.prepare_capability_invocation(capability_name, payload, ctx)?;
        let dispatch_kind = match plan.binding.backend_kind {
            PluginBackendKind::InProcess => PluginCapabilityDispatchKind::BuiltinInProcess,
            PluginBackendKind::Process | PluginBackendKind::Command => {
                PluginCapabilityDispatchKind::ExternalProtocol
            },
            PluginBackendKind::Http => PluginCapabilityDispatchKind::ExternalHttp,
        };
        Some(PluginCapabilityInvocationTarget {
            dispatch_kind,
            plan,
        })
    }

    pub fn prepare_capability_invocation(
        &self,
        capability_name: &str,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Option<PluginCapabilityInvocationPlan> {
        let binding = self.capability_binding(capability_name)?;
        let invocation_context = to_plugin_invocation_context(ctx, capability_name);
        let stream = matches!(
            binding.capability.invocation_mode,
            InvocationMode::Streaming
        );
        let invoke_message = InvokeMessage {
            id: invocation_context.request_id.clone(),
            capability: binding.capability.name.to_string(),
            input: payload.clone(),
            context: invocation_context.clone(),
            stream,
        };
        Some(PluginCapabilityInvocationPlan {
            binding,
            payload,
            stream,
            invocation_context,
            invoke_message,
        })
    }

    pub fn capability_binding(&self, capability_name: &str) -> Option<PluginCapabilityBinding> {
        self.descriptors.iter().find_map(|descriptor| {
            descriptor
                .tools
                .iter()
                .find(|tool| tool.name.as_ref() == capability_name)
                .map(|tool| PluginCapabilityBinding {
                    plugin_id: descriptor.plugin_id.clone(),
                    display_name: descriptor.display_name.clone(),
                    source_ref: descriptor.source_ref.clone(),
                    backend_kind: descriptor.source_kind.to_backend_kind(),
                    capability: tool.clone(),
                    runtime_handle: self.runtime_handle_snapshot(&descriptor.plugin_id),
                })
        })
    }

    pub fn capability_bindings(&self) -> Vec<PluginCapabilityBinding> {
        self.descriptors
            .iter()
            .flat_map(|descriptor| {
                descriptor.tools.iter().map(|tool| PluginCapabilityBinding {
                    plugin_id: descriptor.plugin_id.clone(),
                    display_name: descriptor.display_name.clone(),
                    source_ref: descriptor.source_ref.clone(),
                    backend_kind: descriptor.source_kind.to_backend_kind(),
                    capability: tool.clone(),
                    runtime_handle: self.runtime_handle_snapshot(&descriptor.plugin_id),
                })
            })
            .collect()
    }

    pub fn runtime_handle_snapshot(&self, plugin_id: &str) -> Option<PluginRuntimeHandleSnapshot> {
        if let Some(handle) = self
            .builtin_backends
            .iter()
            .find(|handle| handle.plugin_id == plugin_id)
        {
            let health = self.backend_health.report(plugin_id);
            return Some(PluginRuntimeHandleSnapshot {
                plugin_id: handle.plugin_id.clone(),
                backend_kind: PluginBackendKind::InProcess,
                started_at_ms: handle.started_at_ms,
                shutdown_requested: false,
                health: health.map(|report| report.health.clone()),
                message: health.and_then(|report| report.message.clone()),
                local_protocol_version: None,
                remote_negotiated: false,
            });
        }

        self.external_backends
            .iter()
            .find(|handle| handle.plugin_id == plugin_id)
            .map(|handle| {
                let health = self.backend_health.report(plugin_id);
                PluginRuntimeHandleSnapshot {
                    plugin_id: handle.plugin_id.clone(),
                    backend_kind: PluginBackendKind::Process,
                    started_at_ms: handle.started_at_ms,
                    shutdown_requested: health
                        .map(|report| report.shutdown_requested)
                        .unwrap_or(false),
                    health: health.map(|report| report.health.clone()),
                    message: health.and_then(|report| report.message.clone()),
                    local_protocol_version: handle
                        .protocol_state()
                        .map(|state| state.local_initialize.protocol_version.clone()),
                    remote_negotiated: handle.remote_handshake_summary().is_some(),
                }
            })
    }

    pub fn runtime_handle_snapshots(&self) -> Vec<PluginRuntimeHandleSnapshot> {
        self.runtime_catalog
            .plugin_ids
            .iter()
            .filter_map(|plugin_id| self.runtime_handle_snapshot(plugin_id))
            .collect()
    }

    pub fn runtime_handle(&self, plugin_id: &str) -> Option<PluginRuntimeHandleRef<'_>> {
        if let Some(handle) = self
            .builtin_backends
            .iter()
            .find(|handle| handle.plugin_id == plugin_id)
        {
            return Some(PluginRuntimeHandleRef::Builtin(handle));
        }

        self.external_backends
            .iter()
            .find(|handle| handle.plugin_id == plugin_id)
            .map(PluginRuntimeHandleRef::External)
    }

    pub fn refresh_external_backend_health(&mut self, host: &PluginHost) -> Result<()> {
        let mut reports = self
            .builtin_backends
            .iter()
            .map(BuiltinPluginRuntimeHandle::health_report)
            .collect::<Vec<_>>();
        reports.extend(host.external_backend_health_reports(&mut self.external_backends)?);
        self.backend_health = ExternalBackendHealthCatalog::from_reports(reports);
        self.refresh_runtime_catalog();
        Ok(())
    }

    pub fn plugin_descriptor(&self, plugin_id: &str) -> Option<&PluginDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.plugin_id == plugin_id)
    }

    define_descriptor_lookup!(tool_descriptor, tools, name, CapabilityWireDescriptor);
    define_descriptor_lookup!(hook_descriptor, hooks, hook_id, HookDescriptor);
    define_descriptor_lookup!(
        provider_descriptor,
        providers,
        provider_id,
        ProviderDescriptor
    );
    define_descriptor_lookup!(
        resource_descriptor,
        resources,
        resource_id,
        ResourceDescriptor
    );
    define_descriptor_lookup!(command_descriptor, commands, command_id, CommandDescriptor);
    define_descriptor_lookup!(theme_descriptor, themes, theme_id, ThemeDescriptor);
    define_descriptor_lookup!(prompt_descriptor, prompts, prompt_id, PromptDescriptor);
    define_descriptor_lookup!(skill_descriptor, skills, skill_id, SkillDescriptor);

    pub fn refresh_negotiated_plugins(&mut self) {
        self.negotiated_plugins
            .refresh_from_external_backends(&self.external_backends);
        self.refresh_runtime_catalog();
    }

    pub fn refresh_runtime_catalog(&mut self) {
        self.runtime_catalog = ActivePluginRuntimeCatalog::from_reload(self);
    }

    pub fn record_remote_initialize(
        &mut self,
        plugin_id: &str,
        remote_initialize: InitializeResultData,
    ) -> Result<()> {
        let backend = self
            .external_backends
            .iter_mut()
            .find(|backend| backend.plugin_id == plugin_id)
            .ok_or_else(|| {
                AstrError::Validation(format!(
                    "reload 结果中不存在 external plugin '{}'",
                    plugin_id
                ))
            })?;
        backend.record_remote_initialize(remote_initialize)?;
        self.refresh_negotiated_plugins();
        Ok(())
    }

    async fn execute_protocol_capability_live(
        &mut self,
        target: PluginCapabilityInvocationTarget,
    ) -> Result<CapabilityExecutionResult> {
        let plugin_id = target.plan.binding.plugin_id.clone();
        let needs_remote_initialize = self
            .external_backends
            .iter()
            .find(|backend| backend.plugin_id == plugin_id)
            .ok_or_else(|| {
                AstrError::Validation(format!(
                    "reload 结果中不存在 external plugin '{}'",
                    plugin_id
                ))
            })?
            .protocol_state()
            .map(|state| state.remote_initialize.is_none())
            .unwrap_or(true);

        if needs_remote_initialize {
            let remote_initialize = {
                let backend = self
                    .external_backends
                    .iter_mut()
                    .find(|backend| backend.plugin_id == plugin_id)
                    .ok_or_else(|| {
                        AstrError::Validation(format!(
                            "reload 结果中不存在 external plugin '{}'",
                            plugin_id
                        ))
                    })?;
                backend.initialize_remote().await?.clone()
            };
            self.record_remote_initialize(&plugin_id, remote_initialize)?;
        }

        let runtime_handle = self.runtime_handle_snapshot(&plugin_id).ok_or_else(|| {
            AstrError::Validation(format!(
                "能力 '{}' 的 external backend 缺少运行时快照",
                target.plan.binding.capability.name
            ))
        })?;
        let dispatch = PluginCapabilityProtocolDispatch {
            runtime_handle,
            target,
        };
        let execution_result = {
            let backend = self
                .external_backends
                .iter_mut()
                .find(|backend| backend.plugin_id == plugin_id)
                .ok_or_else(|| {
                    AstrError::Validation(format!(
                        "reload 结果中不存在 external plugin '{}'",
                        plugin_id
                    ))
                })?;
            if dispatch.target.plan.stream {
                backend
                    .invoke_stream(&dispatch.target.plan.invoke_message)
                    .await
                    .map(PluginCapabilityProtocolExecution::Stream)
            } else {
                backend
                    .invoke_unary(&dispatch.target.plan.invoke_message)
                    .await
                    .map(PluginCapabilityProtocolExecution::Unary)
            }
        };
        match execution_result {
            Ok(execution) => dispatch.into_execution_result_from_dispatch(execution),
            Err(error) => {
                let _ = self.refresh_backend_health_from_runtime_handles();
                self.mark_backend_unavailable(
                    &plugin_id,
                    Some(format!("live protocol invoke failed: {error}")),
                );
                Err(error)
            },
        }
    }

    fn refresh_backend_health_from_runtime_handles(&mut self) -> Result<()> {
        let mut reports = self
            .builtin_backends
            .iter()
            .map(BuiltinPluginRuntimeHandle::health_report)
            .collect::<Vec<_>>();
        reports.extend(
            self.external_backends
                .iter_mut()
                .map(|backend| backend.health_report())
                .collect::<Result<Vec<_>>>()?,
        );
        self.backend_health = ExternalBackendHealthCatalog::from_reports(reports);
        self.refresh_runtime_catalog();
        Ok(())
    }

    fn mark_backend_unavailable(&mut self, plugin_id: &str, message: Option<String>) {
        let started_at_ms = self
            .external_backends
            .iter()
            .find(|backend| backend.plugin_id == plugin_id)
            .map(|backend| backend.started_at_ms)
            .unwrap_or(0);
        let shutdown_requested = self
            .backend_health
            .report(plugin_id)
            .map(|report| report.shutdown_requested)
            .unwrap_or(false);
        let report = PluginBackendHealthReport {
            plugin_id: plugin_id.to_string(),
            health: PluginBackendHealth::Unavailable,
            started_at_ms,
            shutdown_requested,
            message,
        };
        self.backend_health
            .reports
            .retain(|item| item.plugin_id != plugin_id);
        self.backend_health.reports.push(report);
        self.refresh_runtime_catalog();
    }
}
