//! # 策略层
//!
//! `core` 保留 durable event 需要序列化的 prompt layer 枚举。

mod engine;

pub use engine::SystemPromptLayer;
