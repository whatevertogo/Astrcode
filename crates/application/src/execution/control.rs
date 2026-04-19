//! 执行控制参数 re-export。
//!
//! 将 `astrcode_core::ExecutionControl` 直接 re-export，
//! 供 application 各模块统一从 `execution::ExecutionControl` 引入。

pub use astrcode_core::ExecutionControl;
