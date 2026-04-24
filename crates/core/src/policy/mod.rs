//! # Prompt metrics layer
//!
//! `core` 只保留 durable event 需要序列化的 prompt layer 枚举。
//! 策略引擎与治理契约位于 `astrcode-governance-contract`。

mod engine;

pub use engine::SystemPromptLayer;
