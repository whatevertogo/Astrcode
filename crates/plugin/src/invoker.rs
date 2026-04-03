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

use std::sync::Arc;
use std::time::Instant;

use astrcode_core::{
    AstrError, CapabilityContext, CapabilityDescriptor, CapabilityExecutionResult,
    CapabilityInvoker, Result,
};
use astrcode_protocol::plugin::{EventPhase, InvocationContext, WorkspaceRef};
use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{Peer, StreamExecution, Supervisor};

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
    descriptor: CapabilityDescriptor,
    remote_name: String,
}

impl PluginCapabilityInvoker {
    /// 从协议描述符创建调用器。
    ///
    /// `remote_name` 保存原始的能力名称，因为 `descriptor.name` 可能在
    /// 适配过程中被修改（如添加命名空间前缀）。
    pub fn from_protocol_descriptor(peer: Peer, descriptor: CapabilityDescriptor) -> Self {
        Self {
            remote_name: descriptor.name.clone(),
            descriptor,
            peer,
        }
    }
}

#[async_trait]
impl CapabilityInvoker for PluginCapabilityInvoker {
    fn descriptor(&self) -> CapabilityDescriptor {
        self.descriptor.clone()
    }

    /// 执行能力调用。
    ///
    /// 根据能力的 `streaming` 标志选择调用模式：
    /// - 流式模式：通过 `invoke_stream` 获取 `StreamExecution`，
    ///   然后收集所有 delta 事件直到终端事件
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

        if self.descriptor.streaming {
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
            finish_stream_invocation(self.descriptor.name.clone(), &mut stream, started_at).await
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
            Ok(CapabilityExecutionResult {
                capability_name: self.descriptor.name.clone(),
                success,
                output: result.output,
                error,
                metadata: Some(result.metadata),
                duration_ms: started_at.elapsed().as_millis() as u64,
                truncated: false,
            })
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
            .map(|descriptor| {
                Arc::new(PluginCapabilityInvoker::from_protocol_descriptor(
                    self.peer(),
                    descriptor,
                )) as Arc<dyn CapabilityInvoker>
            })
            .collect()
    }

    /// 获取此插件声明的核心能力描述符列表。
    ///
    /// 与 `capability_invokers()` 不同，此方法返回原始的描述符，
    /// 不包装为调用器。用于向宿主展示插件提供了哪些能力。
    pub fn core_capabilities(&self) -> Vec<CapabilityDescriptor> {
        self.remote_initialize().capabilities.clone()
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
            EventPhase::Started => {}
            EventPhase::Delta => {
                deltas.push(json!({
                    "event": event.event,
                    "payload": event.payload,
                    "seq": event.seq,
                }));
            }
            EventPhase::Completed => {
                return Ok(CapabilityExecutionResult {
                    capability_name,
                    success: true,
                    output: event.payload,
                    error: None,
                    metadata: Some(json!({ "streamEvents": deltas })),
                    duration_ms: started_at.elapsed().as_millis() as u64,
                    truncated: false,
                });
            }
            EventPhase::Failed => {
                let error = event
                    .error
                    .map(|value| value.message)
                    .unwrap_or_else(|| "stream invocation failed".to_string());
                return Ok(CapabilityExecutionResult {
                    capability_name,
                    success: false,
                    output: Value::Null,
                    error: Some(error),
                    metadata: Some(json!({ "streamEvents": deltas })),
                    duration_ms: started_at.elapsed().as_millis() as u64,
                    truncated: false,
                });
            }
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
        session_id: Some(ctx.session_id.clone()),
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
