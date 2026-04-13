//! 能力路由与权限检查。
//!
//! 本模块负责将能力调用请求路由到对应的处理器，并在执行前进行权限验证。
//!
//! ## 核心组件
//!
//! - `CapabilityHandler`: 能力处理器的 trait，每个实现代表一个可被调用的能力
//! - `PermissionChecker`: 权限检查 trait，决定是否允许某个能力在特定上下文中执行
//! - `CapabilityRouter`: 能力路由器，维护能力注册表并执行路由+权限检查
//!
//! ## 调用流程
//!
//! 1. 调用方通过 `router.invoke(capability_name, ...)` 发起调用
//! 2. 路由器查找对应的 handler
//! 3. 验证 profile 兼容性（能力的 profiles 必须包含上下文的 profile）
//! 4. 执行权限检查
//! 5. 调用 handler.invoke() 执行实际逻辑

use std::{collections::BTreeMap, sync::Arc};

use astrcode_core::{AstrError, CancelToken, CapabilitySpec, Result};
use astrcode_protocol::plugin::{CapabilityDescriptor, InvocationContext};
use async_trait::async_trait;
use serde_json::Value;

use crate::{EventEmitter, capability_mapping::spec_to_descriptor};

/// 能力处理器 trait。
///
/// 每个实现代表一个可被插件或宿主调用的能力。
/// 能力通过 `capability_spec()` 声明元数据（名称、类型、输入输出 schema 等），
/// 通过 `invoke()` 执行实际逻辑。
///
/// # 线程安全
///
/// 需要 `Send + Sync`，因为路由器可能在多线程环境中并发调用。
#[async_trait]
pub trait CapabilityHandler: Send + Sync {
    fn capability_spec(&self) -> CapabilitySpec;

    async fn invoke(
        &self,
        input: Value,
        context: InvocationContext,
        events: EventEmitter,
        cancel: CancelToken,
    ) -> Result<Value>;
}

/// 权限检查器 trait。
///
/// 在能力执行前进行权限验证。实现可以根据能力描述和调用上下文
/// 决定是否允许执行。例如：检查用户是否授权了文件系统访问、
/// 是否在沙箱环境中运行等。
///
/// 默认实现 `AllowAllPermissionChecker` 允许所有请求，
/// 生产环境应替换为更严格的检查器。
pub trait PermissionChecker: Send + Sync {
    fn check(&self, capability: &CapabilitySpec, context: &InvocationContext) -> Result<()>;
}

/// 允许所有请求的权限检查器。
///
/// 用于开发环境或不需要权限隔离的场景。
/// 生产环境应使用更严格的实现。
#[derive(Debug, Default)]
pub struct AllowAllPermissionChecker;

impl PermissionChecker for AllowAllPermissionChecker {
    fn check(&self, _capability: &CapabilitySpec, _context: &InvocationContext) -> Result<()> {
        Ok(())
    }
}

/// 能力路由器——维护能力注册表并执行路由+权限检查。
///
/// # 职责
///
/// - 注册能力处理器（通过 `register` 或 `register_arc`）
/// - 查询已注册的能力列表
/// - 根据能力名称路由调用请求
/// - 验证 profile 兼容性
/// - 执行权限检查
///
/// # 内部实现
///
/// 使用 `BTreeMap` 存储处理器以保证确定性遍历顺序，
/// 这对于能力列表的序列化一致性很重要。
pub struct CapabilityRouter {
    handlers: BTreeMap<String, Arc<dyn CapabilityHandler>>,
    permission_checker: Arc<dyn PermissionChecker>,
}

impl Default for CapabilityRouter {
    /// 创建使用 `AllowAllPermissionChecker` 的默认路由器。
    fn default() -> Self {
        Self::new(Arc::new(AllowAllPermissionChecker))
    }
}

impl CapabilityRouter {
    /// 创建使用指定权限检查器的路由器。
    pub fn new(permission_checker: Arc<dyn PermissionChecker>) -> Self {
        Self {
            handlers: BTreeMap::new(),
            permission_checker,
        }
    }

    /// 注册一个能力处理器。
    ///
    /// # 验证
    ///
    /// - 检查 `CapabilitySpec` 的合法性（名称格式、必填字段等）
    /// - 检查是否已有同名能力（不允许重复注册）
    ///
    /// # 错误
    ///
    /// - descriptor 验证失败返回 `Validation` 错误
    /// - 重复注册返回 `Validation` 错误
    pub fn register<H>(&mut self, handler: H) -> Result<()>
    where
        H: CapabilityHandler + 'static,
    {
        self.register_arc(Arc::new(handler))
    }

    /// 注册一个 `Arc` 包装的能力处理器。
    ///
    /// 与 `register()` 功能相同，但允许调用方自行管理 handler 的 `Arc`，
    /// 适用于需要在多处共享同一个 handler 实例的场景。
    pub fn register_arc(&mut self, handler: Arc<dyn CapabilityHandler>) -> Result<()> {
        let spec = handler.capability_spec();
        spec.validate().map_err(|error| {
            AstrError::Validation(format!(
                "invalid capability spec '{}': {}",
                spec.name, error
            ))
        })?;
        if self.handlers.contains_key(spec.name.as_str()) {
            return Err(AstrError::Validation(format!(
                "duplicate capability registration: {}",
                spec.name
            )));
        }
        self.handlers.insert(spec.name.to_string(), handler);
        Ok(())
    }

    /// 获取所有已注册能力的描述符列表。
    ///
    /// 返回顺序由内部 `BTreeMap` 的键顺序决定（按能力名称字典序）。
    pub fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>> {
        self.handlers
            .values()
            .map(|handler| {
                let spec = handler.capability_spec();
                spec_to_descriptor(&spec).map_err(|error| {
                    AstrError::Validation(format!(
                        "failed to project capability spec '{}' to descriptor: {}",
                        spec.name, error
                    ))
                })
            })
            .collect()
    }

    /// 调用指定能力。
    ///
    /// # 执行流程
    ///
    /// 1. 查找能力 handler，不存在则返回 `Validation` 错误
    /// 2. 验证 profile 兼容性（能力的 profiles 必须包含上下文的 profile）
    /// 3. 执行权限检查
    /// 4. 调用 handler.invoke() 执行实际逻辑
    ///
    /// # 参数
    ///
    /// * `capability` - 能力名称（如 `tool.echo`）
    /// * `input` - 输入参数，需符合能力的 `input_schema`
    /// * `context` - 调用上下文，包含 session、workspace、profile 等信息
    /// * `events` - 事件发射器，用于流式输出
    /// * `cancel` - 取消令牌，用于中途取消
    pub async fn invoke(
        &self,
        capability: &str,
        input: Value,
        context: InvocationContext,
        events: EventEmitter,
        cancel: CancelToken,
    ) -> Result<Value> {
        let handler = self
            .handlers
            .get(capability)
            .ok_or_else(|| AstrError::Validation(format!("unknown capability '{capability}'")))?;
        let spec = handler.capability_spec();
        self.validate_profile(&spec, &context)?;
        self.permission_checker.check(&spec, &context)?;
        handler.invoke(input, context, events, cancel).await
    }

    /// 验证能力是否支持调用上下文的 profile。
    ///
    /// 如果能力没有声明任何 profiles（空列表），则认为支持所有 profile。
    /// 否则，上下文的 profile 必须在能力的 profiles 列表中。
    fn validate_profile(&self, spec: &CapabilitySpec, context: &InvocationContext) -> Result<()> {
        if spec.profiles.is_empty() || spec.profiles.contains(&context.profile) {
            return Ok(());
        }
        Err(AstrError::Validation(format!(
            "capability '{}' does not support profile '{}'",
            spec.name, context.profile
        )))
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::CapabilityKind;
    use serde_json::json;

    use super::*;

    struct SampleHandler;

    #[async_trait]
    impl CapabilityHandler for SampleHandler {
        fn capability_spec(&self) -> CapabilitySpec {
            CapabilitySpec::builder("tool.sample", CapabilityKind::Tool)
                .description("sample")
                .schema(json!({ "type": "object" }), json!({ "type": "object" }))
                .profiles(["coding"])
                .build()
                .expect("sample capability spec should build")
        }

        async fn invoke(
            &self,
            input: Value,
            _context: InvocationContext,
            _events: EventEmitter,
            _cancel: CancelToken,
        ) -> Result<Value> {
            Ok(input)
        }
    }

    struct DenyChecker;

    impl PermissionChecker for DenyChecker {
        fn check(&self, _capability: &CapabilitySpec, _context: &InvocationContext) -> Result<()> {
            Err(AstrError::Validation("denied by checker".to_string()))
        }
    }

    fn context(profile: &str) -> InvocationContext {
        InvocationContext {
            request_id: "req-1".to_string(),
            trace_id: None,
            session_id: None,
            caller: None,
            workspace: None,
            deadline_ms: None,
            budget: None,
            profile: profile.to_string(),
            profile_context: Value::Null,
            metadata: Value::Null,
        }
    }

    #[tokio::test]
    async fn router_rejects_unsupported_profile() {
        let mut router = CapabilityRouter::default();
        router.register(SampleHandler).expect("register handler");

        let error = router
            .invoke(
                "tool.sample",
                json!({}),
                context("workflow"),
                EventEmitter::noop(),
                CancelToken::new(),
            )
            .await
            .expect_err("unsupported profile should fail");
        assert!(matches!(error, AstrError::Validation(_)));
        assert!(error.to_string().contains("does not support profile"));
    }

    #[tokio::test]
    async fn router_applies_permission_checker_before_invocation() {
        let mut router = CapabilityRouter::new(Arc::new(DenyChecker));
        router.register(SampleHandler).expect("register handler");

        let error = router
            .invoke(
                "tool.sample",
                json!({}),
                context("coding"),
                EventEmitter::noop(),
                CancelToken::new(),
            )
            .await
            .expect_err("permission checker should fail");
        assert!(matches!(error, AstrError::Validation(_)));
        assert!(error.to_string().contains("denied by checker"));
    }
}
