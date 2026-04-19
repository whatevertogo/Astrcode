//! # 应用层端口（Port）
//!
//! 定义 application 层与外部系统交互的 trait 契约，实现依赖反转：
//! - `AppKernelPort`：`App` 依赖的 kernel 控制面
//! - `AgentKernelPort`：Agent 编排子域扩展的 kernel 端口
//! - `AppSessionPort`：`App` 依赖的 session-runtime 稳定端口
//! - `AgentSessionPort`：Agent 编排子域扩展的 session 端口
//! - `ComposerSkillPort`：composer 输入补全的 skill 查询端口

mod agent_kernel;
mod agent_session;
mod app_kernel;
mod app_session;
mod composer_skill;
mod session_submission;

pub use agent_kernel::AgentKernelPort;
pub use agent_session::AgentSessionPort;
pub use app_kernel::AppKernelPort;
pub use app_session::AppSessionPort;
pub use composer_skill::{ComposerResolvedSkill, ComposerSkillPort};
pub use session_submission::AppAgentPromptSubmission;
