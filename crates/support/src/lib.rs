//! Astrcode 共享支持层。
//!
//! 这个 crate 只承载跨多个 crate 共享、但不应继续滞留在 `core`
//! 的宿主环境辅助能力。当前仅包含 `hostpaths` 子域。

pub mod hostpaths;
pub mod shell;
pub mod tool_results;
