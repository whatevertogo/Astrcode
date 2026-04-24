//! # 能力注册表
//!
//! 本模块定义了能力路由器的抽象，用于将能力调用分派到具体的执行器。
//!
//! ## 核心概念
//!
//! - **CapabilityInvoker**: 能力调用器的统一接口
//! - **CapabilityRouter**: 能力路由器，根据名称分派调用

pub mod router;

pub use router::{
    CapabilityContext, CapabilityExecutionResult, CapabilityInvoker, ExecutionOwner, ToolEventSink,
};
