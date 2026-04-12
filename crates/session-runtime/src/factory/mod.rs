//! 会话执行构造参数入口。
//!
//! `build_agent_loop` 与 `LoopRuntimeDeps` 的完整实现依赖
//! adapter-llm / adapter-prompt / adapter-skills 等外部 crate，
//! 留在 `runtime-agent-loop` 直到 Phase 10 组合根阶段统一接线。
//!
//! 此处仅定义接口占位，便于 `session-runtime` 内部模块引用。

/// 会话执行构造依赖（占位）。
///
/// Phase 10 组合根接线时将填充实际依赖字段。
#[derive(Debug, Clone, Default)]
pub struct LoopRuntimeDeps;
