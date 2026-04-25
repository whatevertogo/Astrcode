//! # 治理模式子域（Governance Mode）
//!
//! 管理 session 可用的治理模式（code / plan / review 及插件扩展 mode）。
//!
//! 三个子模块各司其职：
//! - `catalog`：模式注册目录，支持内置 + 插件扩展，可热替换插件 mode
//! - `compiler`：将 `GovernanceModeSpec` 编译为治理 compile 产物 `ResolvedTurnEnvelope`
//! - `validator`：校验 mode 之间的合法转换
//!
//! 注意：`ResolvedTurnEnvelope` 虽沿用旧名，但这里只表达 compile 结果；runtime/session/control
//! 绑定后的最终治理快照 owner 在 `governance_surface` 子域。

pub(crate) mod builtin_prompts;
mod catalog;
mod compiler;
mod validator;

pub(crate) use catalog::builtin_mode_specs;
pub use compiler::{CompiledModeEnvelope, compile_mode_envelope, compile_mode_envelope_for_child};
pub use validator::validate_mode_transition;
