//! Agent 执行子域。
//!
//! 承接根代理执行 (`execute_root_agent`) 和子代理执行 (`launch_subagent`)。
//! `App` 通过薄 façade 委托到此子域，避免把执行逻辑堆进根文件。

mod control;
mod profiles;
mod root;
mod subagent;

pub use control::ExecutionControl;
pub use profiles::{ProfileProvider, ProfileResolutionService};
pub use root::{RootExecutionRequest, execute_root_agent};
pub use subagent::{SubagentExecutionRequest, launch_subagent};
