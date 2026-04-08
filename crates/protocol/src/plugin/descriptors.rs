// Re-export capability descriptor types from core crate (authoritative definition).
// Protocol crate 保留 re-export 以保持向后兼容性，所有下游消费者
// 通过 core 或 protocol 都能获取相同的类型。

pub use astrcode_core::capability::{
    BudgetHint, CallerRef, CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind,
    DescriptorBuildError, FilterDescriptor, HandlerDescriptor, InvocationContext, PeerDescriptor,
    PeerRole, PermissionHint, ProfileDescriptor, SideEffectLevel, StabilityLevel,
    TriggerDescriptor, WorkspaceRef,
};
