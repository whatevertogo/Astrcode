//! 插件能力调用器。
//!
//! 本模块实现 `CapabilityInvoker` trait 的插件版本，
//! 将 core 层的能力调用请求转换为插件协议的 `InvokeMessage`。
//!
//! ## 职责
//!
//! - 将 `CapabilityContext` 转换为插件协议的 `InvocationContext`
//! - 根据能力是否支持流式选择 `invoke` 或 `invoke_stream`
//! - 将插件返回的结果转换为 `CapabilityExecutionResult`

use std::{sync::Arc, time::Instant};

use astrcode_core::{
    AstrError, CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, CapabilitySpec,
    InvocationMode, Result,
};
use astrcode_protocol::plugin::{
    CapabilityWireDescriptor, EventPhase, InvocationContext, WorkspaceRef,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{Peer, StreamExecution, Supervisor, capability_mapping::wire_descriptor_to_spec};

/// 插件能力的调用器实现。
///
/// 将 `core::CapabilityInvoker` trait 适配到插件协议的 `Peer`。
/// 每个实例对应一个远程插件能力。
///
/// # 架构位置
///
/// ```text
/// Runtime → CapabilityInvoker (此结构体) → Peer → Transport → 插件进程
/// ```
#[derive(Clone)]
pub struct PluginCapabilityInvoker {
    peer: Peer,
    capability_spec: CapabilitySpec,
    remote_name: String,
}

impl PluginCapabilityInvoker {
    /// 从协议描述符创建调用器。
    ///
    /// `remote_name` 保存原始的能力名称，因为 `descriptor.name` 可能在
    /// 适配过程中被修改（如添加命名空间前缀）。
    pub fn from_wire_descriptor(peer: Peer, descriptor: CapabilityWireDescriptor) -> Result<Self> {
        let capability_spec = wire_descriptor_to_spec(&descriptor).map_err(|error| {
            AstrError::Validation(format!(
                "invalid protocol capability wire descriptor '{}': {}",
                descriptor.name, error
            ))
        })?;
        Ok(Self {
            remote_name: descriptor.name.to_string(),
            capability_spec,
            peer,
        })
    }
}

#[async_trait]
impl CapabilityInvoker for PluginCapabilityInvoker {
    fn capability_spec(&self) -> CapabilitySpec {
        self.capability_spec.clone()
    }

    /// 执行能力调用。
    ///
    /// 根据能力的 `streaming` 标志选择调用模式：
    /// - 流式模式：通过 `invoke_stream` 获取 `StreamExecution`， 然后收集所有 delta
    ///   事件直到终端事件
    /// - 一元模式：通过 `invoke` 等待完整结果
    ///
    /// # 返回
    ///
    /// 总是返回 `Ok(CapabilityExecutionResult)`，即使插件调用失败。
    /// 成功与否通过 `CapabilityExecutionResult::success` 字段判断。
    /// 只有在传输层错误时才返回 `Err`。
    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let started_at = Instant::now();
        let invocation = to_invocation_context(ctx);

        if matches!(
            self.capability_spec.invocation_mode,
            InvocationMode::Streaming
        ) {
            let mut stream = self
                .peer
                .invoke_stream(astrcode_protocol::plugin::InvokeMessage {
                    id: invocation.request_id.clone(),
                    capability: self.remote_name.clone(),
                    input: payload,
                    context: invocation,
                    stream: true,
                })
                .await?;
            finish_stream_invocation(
                self.capability_spec.name.to_string(),
                &mut stream,
                started_at,
            )
            .await
        } else {
            let result = self
                .peer
                .invoke(astrcode_protocol::plugin::InvokeMessage {
                    id: invocation.request_id.clone(),
                    capability: self.remote_name.clone(),
                    input: payload,
                    context: invocation,
                    stream: false,
                })
                .await?;
            let (success, error) = if result.success {
                (true, None)
            } else {
                let error = result
                    .error
                    .map(|value| value.message)
                    .unwrap_or_else(|| "plugin invocation failed".to_string());
                (false, Some(error))
            };
            Ok(CapabilityExecutionResult::from_common(
                self.capability_spec.name.to_string(),
                success,
                result.output,
                None,
                astrcode_core::ExecutionResultCommon {
                    error,
                    metadata: Some(result.metadata),
                    duration_ms: started_at.elapsed().as_millis() as u64,
                    truncated: false,
                },
            ))
        }
    }
}

impl Supervisor {
    /// 获取此插件所有能力的调用器列表。
    ///
    /// 每个调用器封装了一个远程插件能力，实现了 `core::CapabilityInvoker` trait，
    /// 可以被 runtime 统一调度。
    pub fn capability_invokers(&self) -> Vec<Arc<dyn CapabilityInvoker>> {
        self.remote_initialize()
            .capabilities
            .iter()
            .cloned()
            .filter_map(|descriptor| {
                match PluginCapabilityInvoker::from_wire_descriptor(self.peer(), descriptor) {
                    Ok(invoker) => Some(Arc::new(invoker) as Arc<dyn CapabilityInvoker>),
                    Err(error) => {
                        log::error!("failed to adapt plugin capability wire descriptor: {error}");
                        None
                    },
                }
            })
            .collect()
    }

    /// 获取此插件声明的 wire 能力描述符列表。
    ///
    /// 与 `capability_invokers()` 不同，此方法返回原始的描述符，
    /// 不包装为调用器。用于向宿主展示插件提供了哪些能力。
    pub fn wire_capabilities(&self) -> Vec<CapabilityWireDescriptor> {
        self.remote_initialize().capabilities.clone()
    }

    /// 获取此插件声明的 skill 列表。
    ///
    /// 返回插件在握手阶段通过 `InitializeResultData.skills` 声明的 skill。
    /// 调用方负责将这些声明转换为内部的 `SkillSpec`。
    pub fn declared_skills(&self) -> Vec<astrcode_protocol::plugin::SkillDescriptor> {
        self.remote_initialize().skills.clone()
    }

    /// 获取此插件声明的治理 mode 列表。
    ///
    /// 返回插件在握手阶段通过 `InitializeResultData.modes` 声明的 mode。
    /// 调用方负责决定如何校验并注册这些 mode。
    pub fn declared_modes(&self) -> Vec<astrcode_core::GovernanceModeSpec> {
        self.remote_initialize().modes.clone()
    }
}

/// 完成流式调用并收集结果。
///
/// 从 `StreamExecution` 中读取所有事件，收集 delta 事件到 `streamEvents` 元数据中，
/// 直到收到终端事件（Completed 或 Failed）。
///
/// # 错误处理
///
/// 如果 channel 关闭但未收到终端事件，返回 `Internal` 错误。
/// 这通常意味着插件异常退出或传输层断裂。
async fn finish_stream_invocation(
    capability_name: String,
    stream: &mut StreamExecution,
    started_at: Instant,
) -> Result<CapabilityExecutionResult> {
    let mut deltas = Vec::new();

    while let Some(event) = stream.recv().await {
        match event.phase {
            EventPhase::Started => {},
            EventPhase::Delta => {
                deltas.push(json!({
                    "event": event.event,
                    "payload": event.payload,
                    "seq": event.seq,
                }));
            },
            EventPhase::Completed => {
                return Ok(CapabilityExecutionResult::from_common(
                    capability_name,
                    true,
                    event.payload,
                    None,
                    astrcode_core::ExecutionResultCommon::success(
                        Some(json!({ "streamEvents": deltas })),
                        started_at.elapsed().as_millis() as u64,
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
                    capability_name,
                    false,
                    Value::Null,
                    None,
                    astrcode_core::ExecutionResultCommon::failure(
                        error,
                        Some(json!({ "streamEvents": deltas })),
                        started_at.elapsed().as_millis() as u64,
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

fn to_invocation_context(ctx: &CapabilityContext) -> InvocationContext {
    let working_dir = ctx.working_dir.to_string_lossy().into_owned();
    InvocationContext {
        request_id: ctx
            .request_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        trace_id: ctx.trace_id.clone(),
        session_id: Some(ctx.session_id.to_string()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some(working_dir.clone()),
            repo_root: Some(working_dir),
            branch: None,
            metadata: Value::Null,
        }),
        deadline_ms: None,
        budget: None,
        profile: ctx.profile.clone(),
        profile_context: ctx.profile_context.clone(),
        metadata: ctx.metadata.clone(),
    }
}
