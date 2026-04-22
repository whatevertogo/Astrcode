//! # Astrcode 内置工具 + Agent 协作工具
//!
//! 本库实现 Astrcode 编码代理（agent）的本地工具集：
//! - **core builtin tools**（`builtin_tools`）：readFile、writeFile、editFile、apply_patch、
//!   listDir、findFiles、grep、shell、tool_search、Skill
//! - **agent tools**（`agent_tools`）：spawn、send、observe、close
//!
//! 所有工具均实现 `astrcode_core::Tool` trait。
//!
//! ## 架构约束
//!
//! - 本 crate 仅依赖 `astrcode-core`，不依赖 `runtime` 或其他业务 crate
//! - 所有工具通过 `Tool` trait 统一接口暴露，由 `runtime` 层统一调度
//! - 工具执行结果包含结构化 metadata，供前端渲染（如终端视图、diff 视图）

pub mod agent_tools;
pub mod builtin_tools;

pub use agent_tools::{
    CloseAgentTool, CollaborationExecutor, ObserveAgentTool, SendAgentTool, SpawnAgentTool,
};

#[cfg(test)]
pub(crate) mod test_support;
