//! # 运行时接口
//!
//! 定义了运行时组件的抽象接口，用于管理 LLM 连接和生命周期。
//!
//! ## 核心接口
//!
//! - `RuntimeHandle`: 运行时主句柄
//! - `ManagedRuntimeComponent`: 可被协调器管理的子组件
//! - `RuntimeCoordinator`: 统一管理运行时实例、插件和能力列表

mod coordinator;
mod traits;

pub use coordinator::RuntimeCoordinator;
pub use traits::{
    ExecutionAccepted, ExecutionOrchestrationBoundary, LiveSubRunControlBoundary,
    LoopRunnerBoundary, ManagedRuntimeComponent, RuntimeHandle, SessionTruthBoundary,
};
