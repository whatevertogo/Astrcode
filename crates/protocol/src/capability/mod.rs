//! 能力描述符与调用上下文
//!
//! 定义插件系统核心的元数据描述结构，是 host 与插件之间
//! 能力注册、路由、策略决策的基础协议。
//!
//! ## 主要类型
//!
//! - **PeerDescriptor**: 通信对等方的身份信息（ID、角色、版本、支持的 profile）
//! - **CapabilityDescriptor**: 能力的完整描述（名称、类型、schema、权限、副作用级别等）
//! - **CapabilityKind**: 能力类型的强类型包装，避免拼写错误导致路由失败
//! - **HandlerDescriptor**: 事件处理器的描述（触发条件、过滤规则）
//! - **InvocationContext**: 调用时的上下文（调用方、工作区、预算限制等）
//! - **CapabilityDescriptorBuilder**: 构建器模式，用于安全地构造能力描述符
//!
//! ## 设计原则
//!
//! - 能力描述符在插件握手时由插件发送给 host，host 据此进行路由和策略决策
//! - `CapabilityKind` 虽然是字符串包装，但提供了强类型的构造函数（`tool()`, `agent()` 等）
//! - Builder 在 `build()` 时执行完整校验，确保描述符的完整性
//! - 所有字段都有明确的默认值和 serde 注解，保证序列化兼容性

mod descriptors;
mod mapper;

pub use descriptors::{
    BudgetHint, CallerRef, CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind,
    DescriptorBuildError, FilterDescriptor, HandlerDescriptor, InvocationContext, PeerDescriptor,
    PeerRole, PermissionHint, ProfileDescriptor, SideEffectLevel, StabilityLevel,
    TriggerDescriptor, WorkspaceRef,
};
pub use mapper::{CapabilityMappingError, descriptor_to_spec, spec_to_descriptor};
