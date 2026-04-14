//! bootstrap 内部底层依赖别名。
//!
//! 为什么保留这一层：
//! 让 `core` / `kernel` / `session-runtime` 的直接依赖只集中在少数入口文件，
//! 其他装配模块统一通过本地 facade 引用，避免 import 散点继续扩散。

pub(crate) use astrcode_core as core;
pub(crate) use astrcode_kernel as kernel;
pub(crate) use astrcode_session_runtime as session_runtime;
