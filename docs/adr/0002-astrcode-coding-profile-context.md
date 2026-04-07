# ADR-0002: Freeze Coding Profile Context Boundary

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 面向编码场景，但协议层不能退化为只服务代码编辑的专用协议。若把 coding 语义直接写死在顶层消息中，会破坏协议通用性，并让 UI、runtime 和 plugin 的临时字段持续污染公共边界。

## Decision

冻结调用上下文边界，区分通用上下文与 profile 专属上下文。

- `InvocationContext` 只承载通用字段，例如 `request_id`、`trace_id`、`session_id`、`caller`、`workspace`、`deadline_ms`、`budget`、`profile`、`metadata`。
- `profile_context` 承载 profile 专属语义；`coding` 是首个官方 profile。
- `coding` profile 的编辑器与仓库语义保留在 `profile_context` 中，不提升为通用顶层字段。
- `workspace` 表达跨 profile 通用的工作区引用；编辑器态与当前编码态信息由 `profile_context` 表达。
- 未来新增 coding 专属字段，应通过 `coding` profile 自身演进，而不是修改通用顶层结构。

## Consequences

- 协议保持 coding-first，同时保留扩展到其他 profile 的空间。
- 通用协议边界更稳定，UI 或运行时特有字段更难渗入公共协议。
- 插件和 SDK 需要明确区分通用上下文与 `profile_context` 的职责。
