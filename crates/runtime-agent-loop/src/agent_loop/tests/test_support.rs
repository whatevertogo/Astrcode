//! # 测试支持 (Test Support)
//!
//! 提供测试中使用的工具函数和辅助类型：
//! - `empty_capabilities()` - 空能力路由器
//! - `capabilities_from_tools()` - 从工具列表构建能力路由器
//! - `TestEnvGuard` - 测试环境变量保护（复用 core 的实现）

use astrcode_core::Tool;
pub(crate) use astrcode_core::test_support::TestEnvGuard;
use astrcode_runtime_registry::{CapabilityRouter, ToolCapabilityInvoker};

/// 将工具装箱为 `Box<dyn Tool>`，用于构建能力路由器。
pub(crate) fn boxed_tool(tool: impl Tool + 'static) -> Box<dyn Tool> {
    Box::new(tool)
}

/// 创建一个空的能力路由器，不包含任何工具。
pub(crate) fn empty_capabilities() -> CapabilityRouter {
    CapabilityRouter::builder()
        .build()
        .expect("empty capability router should build")
}

/// 从工具列表构建能力路由器。
///
/// 将每个 `Tool` 通过 `ToolCapabilityInvoker` 转换为 `CapabilityInvoker`
/// 并注册到路由器中，用于测试中模拟工具环境。
pub(crate) fn capabilities_from_tools(tools: Vec<Box<dyn Tool>>) -> CapabilityRouter {
    let mut builder = CapabilityRouter::builder();
    for tool in tools {
        let invoker = ToolCapabilityInvoker::boxed(tool).expect("tool descriptor should build");
        builder = builder.register_invoker(invoker);
    }
    builder.build().expect("capability router should build")
}
