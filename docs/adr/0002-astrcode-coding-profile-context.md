# ADR-0002: Freeze Coding Profile Context Boundary

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 的协议需要面向多 profile 扩展，但不能把 coding-specific 语义直接写入通用顶层字段。否则通用调用上下文会被编码场景特有字段污染，导致插件、runtime、SDK 和 transport 的边界变脏。

## Decision

把通用调用上下文与 profile 专属上下文明确分离。

- `InvocationContext` 只承载通用字段，例如 `request_id`、`trace_id`、`session_id`、`caller`、`workspace`、`deadline_ms`、`budget`、`profile`、`metadata` 等。
- `InvocationContext.profile_context` 保留给 profile 专属语义。当前 `coding` profile 是首个官方 profile，其他 profile 可并行演进。
- `coding` profile 的编辑器态、工作区上下文、仓库语义等信息应该放在 `profile_context`，而不是提升为 `InvocationContext` 的通用顶层字段。
- `WorkspaceRef`、`PeerDescriptor` 等通用引用继续保留在通用 `InvocationContext` 中。
- profile 专属字段的语义由对应 profile 的实现方定义，而不是由协议层把它们混进通用上下文。

## Consequences

- 协议保持 coding-first，同时为其他 profile 留出干净扩展空间。
- 通用协议边界更稳定，UI 或运行时特有字段更难渗入公共调用语义。
- 插件、runtime 和 SDK 需要在处理 `InvocationContext.profile_context` 时明确其 profile 语义责任。
