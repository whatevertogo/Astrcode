//! # 测试支持 (Test Support)
//!
//! 提供测试中使用的工具函数和辅助类型：
//! - `empty_capabilities()` - 空能力路由器
//! - `capabilities_from_tools()` - 从工具注册表构建能力路由器
//! - `TestEnvGuard` - 测试环境变量保护（复用 core 的实现）

use astrcode_core::{CapabilityRouter, ToolRegistry};

pub(crate) use astrcode_core::test_support::TestEnvGuard;

/// 创建一个空的能力路由器，不包含任何工具。
///
/// 用于测试中需要隔离工具调用的场景。
pub(crate) fn empty_capabilities() -> CapabilityRouter {
    CapabilityRouter::builder()
        .build()
        .expect("empty capability router should build")
}

/// 从工具注册表构建能力路由器。
///
/// 将 `ToolRegistry` 中的所有工具转换为 `CapabilityInvoker`
/// 并注册到路由器中，用于测试中模拟工具环境。
pub(crate) fn capabilities_from_tools(tools: ToolRegistry) -> CapabilityRouter {
    let mut builder = CapabilityRouter::builder();
    for invoker in tools
        .into_capability_invokers()
        .expect("tool descriptors should build")
    {
        builder = builder.register_invoker(invoker);
    }
    builder
        .build()
        .expect("tool-derived capability router should build")
}
