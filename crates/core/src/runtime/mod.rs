//! # 运行时接口
//!
//! 定义了运行时组件的抽象接口，用于管理 LLM 连接和生命周期。
//!
//! ## 核心接口
//!
//! - `RuntimeHandle`: 运行时主句柄
//! - `ManagedRuntimeComponent`: 可被组合根管理的子组件

mod traits;

pub use traits::{
    ExecutionAccepted, ExecutionOrchestrationBoundary, LiveSubRunControlBoundary,
    LoopRunnerBoundary, ManagedRuntimeComponent, RuntimeHandle, SessionTruthBoundary,
};
