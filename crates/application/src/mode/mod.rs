//! # 治理模式子域（Governance Mode）
//!
//! 管理 session 可用的治理模式（code / plan / review 及插件扩展 mode）。
//!
//! 三个子模块各司其职：
//! - `catalog`：模式注册目录，支持内置 + 插件扩展，可热替换插件 mode
//! - `compiler`：将 `GovernanceModeSpec` 编译为 `ResolvedTurnEnvelope`（工具白名单 + 策略 +
//!   prompt）
//! - `validator`：校验 mode 之间的合法转换

pub(crate) mod builtin_prompts;
mod catalog;
mod compiler;
mod validator;

pub use catalog::{
    BuiltinModeCatalog, ModeCatalog, ModeCatalogEntry, ModeCatalogSnapshot, ModeSummary,
    builtin_mode_catalog,
};
pub use compiler::{
    CompiledModeEnvelope, compile_capability_selector, compile_mode_envelope,
    compile_mode_envelope_for_child,
};
pub use validator::{ModeTransitionDecision, validate_mode_transition};
