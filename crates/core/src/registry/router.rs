//! # 能力路由器
//!
//! 将能力调用分派到具体的执行器（`CapabilityInvoker`）。
//!
//! ## 核心概念
//!
//! - **CapabilityInvoker**: 统一的异步能力调用接口
//! - **CapabilityRouter**: 根据能力名称路由到对应的 invoker
//! - **CapabilityContext**: 调用上下文（会话、工作目录、取消令牌等）
//!
//! ## 动态注册
//!
//! `CapabilityRouter` 支持运行时动态注册能力，用于插件后台加载场景。
//! 使用 `register_invoker` 方法可以在运行时添加新的能力，无需重启服务。
//! 内部使用 `std::sync::RwLock` 保证线程安全，支持同步和异步上下文调用。
//!
//! ## 工具调用适配
//!
//! `CapabilityRouter` 同时提供 `execute_tool` 方法，将通用的能力调用
//! 适配为 LLM 工具调用格式（`ToolExecutionResult`）。这是一种 adapter view，
//! 不是核心能力契约本身。

use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    AgentEventContext, AstrError, CancelToken, CapabilityDescriptor, Result, ToolCallRequest,
    ToolContext, ToolDefinition, ToolEventSink, ToolExecutionResult, ToolOutputDelta,
};

/// 能力调用的上下文信息。
///
/// 从 `ToolContext` 转换而来，携带会话标识、工作目录、取消令牌
/// 以及 profile 上下文等调用期元数据。
#[derive(Clone)]
pub struct CapabilityContext {
    /// 请求唯一标识，用于追踪单次调用链路
    pub request_id: Option<String>,
    /// 分布式追踪标识，关联同一请求的多个子操作
    pub trace_id: Option<String>,
    /// 所属会话标识
    pub session_id: String,
    /// 工作目录，工具执行时的当前路径
    pub working_dir: PathBuf,
    /// 取消令牌，用于外部中断长时间运行的能力调用
    pub cancel: CancelToken,
    /// 当前调用所属 turn。
    pub turn_id: Option<String>,
    /// 当前调用所属 Agent 元数据。
    pub agent: AgentEventContext,
    /// 当前使用的 profile 名称
    pub profile: String,
    /// profile 上下文，包含工作目录、仓库根目录等运行时配置
    pub profile_context: Value,
    /// 调用方自定义元数据
    pub metadata: Value,
    /// 工具增量输出发送器，用于流式推送工具执行结果
    pub tool_output_sender: Option<UnboundedSender<ToolOutputDelta>>,
    /// 工具内部 turn 事件发射器。
    pub event_sink: Option<Arc<dyn ToolEventSink>>,
}

impl fmt::Debug for CapabilityContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapabilityContext")
            .field("request_id", &self.request_id)
            .field("trace_id", &self.trace_id)
            .field("session_id", &self.session_id)
            .field("working_dir", &self.working_dir)
            .field("cancel", &self.cancel)
            .field("turn_id", &self.turn_id)
            .field("agent", &self.agent)
            .field("profile", &self.profile)
            .field("profile_context", &self.profile_context)
            .field("metadata", &self.metadata)
            .field(
                "tool_output_sender",
                &self.tool_output_sender.as_ref().map(|_| "<attached>"),
            )
            .field(
                "event_sink",
                &self.event_sink.as_ref().map(|_| "<attached>"),
            )
            .finish()
    }
}

impl CapabilityContext {
    pub fn from_tool_context(ctx: &ToolContext, request_id: impl Into<Option<String>>) -> Self {
        // 只分配一次：先获取 PathBuf，再从中提取字符串用于 profile_context
        let working_dir = ctx.working_dir().to_path_buf();
        let working_dir_str = working_dir.to_string_lossy().into_owned();
        Self {
            request_id: request_id.into(),
            trace_id: None,
            session_id: ctx.session_id().to_string(),
            working_dir,
            cancel: ctx.cancel().clone(),
            turn_id: ctx.turn_id().map(ToString::to_string),
            agent: ctx.agent_context().clone(),
            profile: "coding".to_string(),
            profile_context: json!({
                "workingDir": working_dir_str,
                "repoRoot": working_dir_str,
                "approvalMode": "inherit"
            }),
            metadata: Value::Null,
            tool_output_sender: ctx.tool_output_sender(),
            event_sink: ctx.event_sink(),
        }
    }
}

/// 能力执行结果。
///
/// 封装单次能力调用的执行状态、输出和耗时，
/// 可转换为 `ToolExecutionResult` 以适配 LLM 工具调用协议。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityExecutionResult {
    /// 能力名称
    pub capability_name: String,
    /// 是否执行成功
    pub success: bool,
    /// 执行输出（JSON 值）
    pub output: Value,
    /// 错误信息（仅在失败时设置）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 执行元数据，如 diff 信息、终端输出类型等
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 输出是否被截断
    #[serde(default)]
    pub truncated: bool,
}

impl CapabilityExecutionResult {
    /// 构造成功结果。
    pub fn ok(capability_name: impl Into<String>, output: Value) -> Self {
        Self {
            capability_name: capability_name.into(),
            success: true,
            output,
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        }
    }

    /// 构造失败结果。
    pub fn failure(
        capability_name: impl Into<String>,
        error: impl Into<String>,
        output: Value,
    ) -> Self {
        Self {
            capability_name: capability_name.into(),
            success: false,
            output,
            error: Some(error.into()),
            metadata: None,
            duration_ms: 0,
            truncated: false,
        }
    }

    /// 将输出格式化为可读文本。
    ///
    /// 字符串直接返回，其他类型序列化为 pretty JSON。
    pub fn output_text(&self) -> String {
        match &self.output {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        }
    }

    /// 转换为 LLM 工具执行结果。
    ///
    /// 将通用的能力执行结果映射为 `ToolExecutionResult`，
    /// 以便前端渲染工具调用卡片。
    pub fn into_tool_execution_result(self, tool_call_id: String) -> ToolExecutionResult {
        let output = self.output_text();
        ToolExecutionResult {
            tool_call_id,
            tool_name: self.capability_name,
            ok: self.success,
            output,
            error: self.error,
            metadata: self.metadata,
            duration_ms: self.duration_ms,
            truncated: self.truncated,
        }
    }
}

/// 能力调用器 trait。
///
/// 所有能力执行器必须实现此 trait，路由器通过它统一分派调用。
#[async_trait]
pub trait CapabilityInvoker: Send + Sync {
    /// 获取能力描述符，包含名称、类型、输入 schema 等元信息。
    fn descriptor(&self) -> CapabilityDescriptor;

    /// 执行能力调用。
    ///
    /// `payload` 为调用参数，`ctx` 携带调用上下文。
    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult>;
}

/// 能力路由器构建器。
///
/// 采用 builder 模式逐步注册能力执行器，
/// 在 `build` 时校验描述符合法性并检测重复注册。
pub struct CapabilityRouterBuilder {
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

impl Default for CapabilityRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityRouterBuilder {
    /// 创建空构建器。
    pub fn new() -> Self {
        Self {
            invokers: Vec::new(),
        }
    }

    /// 注册一个能力执行器。
    ///
    /// 返回自身以支持链式调用。
    pub fn register_invoker(mut self, invoker: Arc<dyn CapabilityInvoker>) -> Self {
        self.invokers.push(invoker);
        self
    }

    /// 构建路由器。
    ///
    /// 校验所有描述符、检测重复注册，并构建工具名称索引。
    pub fn build(self) -> Result<CapabilityRouter> {
        let mut invokers_by_name = HashMap::new();
        let mut order = Vec::new();
        let mut tool_order = Vec::new();

        for invoker in self.invokers {
            let descriptor = invoker.descriptor();
            descriptor.validate().map_err(|error| {
                AstrError::Validation(format!(
                    "invalid capability descriptor '{}': {}",
                    descriptor.name, error
                ))
            })?;
            if invokers_by_name
                .insert(descriptor.name.clone(), Arc::clone(&invoker))
                .is_some()
            {
                return Err(AstrError::Validation(format!(
                    "duplicate capability '{}' registered",
                    descriptor.name
                )));
            }
            if descriptor.kind.is_tool() {
                tool_order.push(descriptor.name.clone());
            }
            order.push(descriptor.name);
        }

        Ok(CapabilityRouter {
            inner: Arc::new(RwLock::new(CapabilityRouterInner {
                invokers_by_name,
                order,
                tool_order,
            })),
        })
    }
}

/// 能力路由器内部状态。
///
/// 被 `RwLock` 保护，支持运行时动态修改。
struct CapabilityRouterInner {
    invokers_by_name: HashMap<String, Arc<dyn CapabilityInvoker>>,
    order: Vec<String>,
    tool_order: Vec<String>,
}

/// 能力路由器。
///
/// 根据能力名称将调用分派到对应的执行器，
/// 同时维护工具可调用能力的有序列表供 LLM 使用。
///
/// ## 动态注册
///
/// 使用 `register_invoker` 方法可以在运行时动态添加能力，
/// 用于插件后台加载场景。
#[derive(Clone)]
pub struct CapabilityRouter {
    inner: Arc<RwLock<CapabilityRouterInner>>,
}

impl CapabilityRouter {
    /// 创建路由器构建器。
    pub fn builder() -> CapabilityRouterBuilder {
        CapabilityRouterBuilder::new()
    }

    /// 创建空的路由器。
    ///
    /// 用于需要后续动态注册能力的场景。
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(CapabilityRouterInner {
                invokers_by_name: HashMap::new(),
                order: Vec::new(),
                tool_order: Vec::new(),
            })),
        }
    }

    /// 动态注册一个能力执行器。
    ///
    /// 用于插件后台加载场景，可以在运行时添加新能力。
    /// 如果能力名称已存在，返回错误。
    pub fn register_invoker(&self, invoker: Arc<dyn CapabilityInvoker>) -> Result<()> {
        let descriptor = invoker.descriptor();
        descriptor.validate().map_err(|error| {
            AstrError::Validation(format!(
                "invalid capability descriptor '{}': {}",
                descriptor.name, error
            ))
        })?;

        let mut inner = self.inner.write().unwrap();
        if inner.invokers_by_name.contains_key(&descriptor.name) {
            return Err(AstrError::Validation(format!(
                "duplicate capability '{}' registered",
                descriptor.name
            )));
        }

        if descriptor.kind.is_tool() {
            inner.tool_order.push(descriptor.name.clone());
        }
        inner.order.push(descriptor.name.clone());
        inner.invokers_by_name.insert(descriptor.name, invoker);
        Ok(())
    }

    /// 批量动态注册能力执行器。
    ///
    /// 用于插件批量加载场景，减少锁竞争。
    /// 如果任一能力名称已存在，全部回滚并返回错误。
    pub fn register_invokers(&self, invokers: Vec<Arc<dyn CapabilityInvoker>>) -> Result<()> {
        let mut inner = self.inner.write().unwrap();

        // 先验证所有描述符
        for invoker in &invokers {
            let descriptor = invoker.descriptor();
            descriptor.validate().map_err(|error| {
                AstrError::Validation(format!(
                    "invalid capability descriptor '{}': {}",
                    descriptor.name, error
                ))
            })?;
            if inner.invokers_by_name.contains_key(&descriptor.name) {
                return Err(AstrError::Validation(format!(
                    "duplicate capability '{}' registered",
                    descriptor.name
                )));
            }
        }

        // 批量注册
        for invoker in invokers {
            let descriptor = invoker.descriptor();
            if descriptor.kind.is_tool() {
                inner.tool_order.push(descriptor.name.clone());
            }
            inner.order.push(descriptor.name.clone());
            inner.invokers_by_name.insert(descriptor.name, invoker);
        }
        Ok(())
    }

    /// 获取所有已注册能力的描述符列表。
    pub fn descriptors(&self) -> Vec<CapabilityDescriptor> {
        let inner = self.inner.read().unwrap();
        inner
            .order
            .iter()
            .filter_map(|name| inner.invokers_by_name.get(name))
            .map(|invoker| invoker.descriptor())
            .collect()
    }

    /// 按名称查询单个能力的描述符。
    pub fn descriptor(&self, name: &str) -> Option<CapabilityDescriptor> {
        let inner = self.inner.read().unwrap();
        inner
            .invokers_by_name
            .get(name)
            .map(|invoker| invoker.descriptor())
    }

    /// 获取工具定义列表（LLM 工具调用视图）。
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.descriptors()
            .into_iter()
            .filter(|descriptor| descriptor.kind.is_tool())
            .map(|descriptor| ToolDefinition {
                name: descriptor.name,
                description: descriptor.description,
                parameters: descriptor.input_schema,
            })
            .collect()
    }

    /// 获取所有工具可调用能力的名称列表。
    pub fn tool_names(&self) -> Vec<String> {
        let inner = self.inner.read().unwrap();
        inner.tool_order.clone()
    }

    /// 基于当前 runtime surface 构建一个仅暴露指定工具的子路由。
    ///
    /// 子 Agent 需要复用同一份 invoker 实现、注册顺序和非工具能力，
    /// 但必须在工具可见性这一层做裁剪，避免 profile allowlist 只停留在 prompt 文本里。
    pub fn subset_for_tools(&self, allowed_tool_names: &[String]) -> Result<Self> {
        let allowed = allowed_tool_names
            .iter()
            .map(|name| name.as_str())
            .collect::<HashSet<_>>();
        let inner = self.inner.read().unwrap();
        let mut builder = CapabilityRouter::builder();

        for name in &inner.order {
            let Some(invoker) = inner.invokers_by_name.get(name) else {
                continue;
            };
            let descriptor = invoker.descriptor();
            if descriptor.kind.is_tool() && !allowed.contains(descriptor.name.as_str()) {
                continue;
            }
            builder = builder.register_invoker(Arc::clone(invoker));
        }

        builder.build()
    }

    /// 执行工具调用。
    ///
    /// 根据工具名称查找对应执行器，校验能力类型后分派调用，
    /// 并将结果转换为 `ToolExecutionResult` 返回。
    pub async fn execute_tool(
        &self,
        call: &ToolCallRequest,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let invoker = {
            let inner = self.inner.read().unwrap();
            inner.invokers_by_name.get(&call.name).cloned()
        };

        let Some(invoker) = invoker else {
            return ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("unknown tool '{}'", call.name)),
                metadata: None,
                duration_ms: 0,
                truncated: false,
            };
        };

        let descriptor = invoker.descriptor();
        if !descriptor.kind.is_tool() {
            return ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("capability '{}' is not tool-callable", call.name)),
                metadata: None,
                duration_ms: 0,
                truncated: false,
            };
        }

        match invoker
            .invoke(
                call.args.clone(),
                &CapabilityContext::from_tool_context(ctx, Some(call.id.clone())),
            )
            .await
        {
            Ok(result) => result.into_tool_execution_result(call.id.clone()),
            Err(error) => ToolExecutionResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(error.to_string()),
                metadata: None,
                duration_ms: 0,
                truncated: false,
            },
        }
    }

    /// 检查能力是否存在。
    pub fn has_capability(&self, name: &str) -> bool {
        let inner = self.inner.read().unwrap();
        inner.invokers_by_name.contains_key(name)
    }

    /// 获取已注册能力数量。
    pub fn capability_count(&self) -> usize {
        let inner = self.inner.read().unwrap();
        inner.invokers_by_name.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::{CapabilityExecutionResult, CapabilityInvoker, CapabilityRouter};
    use crate::{
        CancelToken, CapabilityContext, CapabilityDescriptor, CapabilityKind, Result,
        SideEffectLevel, StabilityLevel, Tool, ToolCallRequest, ToolCapabilityInvoker, ToolContext,
        ToolDefinition, ToolExecutionResult,
    };

    struct FakeTool;

    #[async_trait]
    impl Tool for FakeTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "fake".to_string(),
                description: "fake".to_string(),
                parameters: json!({ "type": "object" }),
            }
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "fake".to_string(),
                ok: true,
                output: "ok".to_string(),
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    struct EchoCapability;

    #[async_trait]
    impl CapabilityInvoker for EchoCapability {
        fn descriptor(&self) -> CapabilityDescriptor {
            CapabilityDescriptor {
                name: "plugin.echo".to_string(),
                kind: CapabilityKind::tool(),
                description: "echo".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                streaming: false,
                concurrency_safe: false,
                compact_clearable: false,
                profiles: vec!["coding".to_string()],
                tags: vec![],
                permissions: vec![],
                side_effect: SideEffectLevel::None,
                stability: StabilityLevel::Stable,
                metadata: Value::Null,
            }
        }

        async fn invoke(
            &self,
            payload: Value,
            _ctx: &CapabilityContext,
        ) -> Result<CapabilityExecutionResult> {
            Ok(CapabilityExecutionResult::ok("plugin.echo", payload))
        }
    }

    fn tool_context() -> ToolContext {
        ToolContext::new(
            "session-1".to_string(),
            std::env::temp_dir(),
            CancelToken::new(),
        )
    }

    #[tokio::test]
    async fn invoker_registered_tools_expose_tool_definitions_and_execute() {
        let router = CapabilityRouter::builder()
            .register_invoker(
                ToolCapabilityInvoker::boxed(Box::new(FakeTool))
                    .expect("tool descriptor should build"),
            )
            .build()
            .expect("router should build");
        assert_eq!(router.tool_names(), vec!["fake".to_string()]);

        let result = router
            .execute_tool(
                &ToolCallRequest {
                    id: "call-1".to_string(),
                    name: "fake".to_string(),
                    args: json!({}),
                },
                &tool_context(),
            )
            .await;
        assert!(result.ok);
        assert_eq!(result.output, "ok");
    }

    #[test]
    fn builder_rejects_duplicate_capabilities() {
        let result = CapabilityRouter::builder()
            .register_invoker(Arc::new(EchoCapability))
            .register_invoker(Arc::new(EchoCapability))
            .build();
        let error = match result {
            Ok(_) => panic!("duplicate capabilities should fail"),
            Err(error) => error,
        };
        assert!(matches!(error, crate::AstrError::Validation(_)));
    }

    #[test]
    fn dynamic_registration_adds_capability() {
        let router = CapabilityRouter::empty();
        assert_eq!(router.capability_count(), 0);

        router
            .register_invoker(Arc::new(EchoCapability))
            .expect("registration should succeed");

        assert_eq!(router.capability_count(), 1);
        assert!(router.has_capability("plugin.echo"));
    }

    #[test]
    fn dynamic_registration_rejects_duplicate() {
        let router = CapabilityRouter::empty();
        router
            .register_invoker(Arc::new(EchoCapability))
            .expect("first registration should succeed");

        let result = router.register_invoker(Arc::new(EchoCapability));
        assert!(result.is_err());
    }
}
