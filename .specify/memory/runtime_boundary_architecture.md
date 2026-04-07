---
name: Runtime Boundary Architecture
description: Five-boundary ownership with single-direction compile-time dependency
type: architecture
---

# Runtime Boundary Architecture

## Compile-Time Dependency Rule (不可违反)

`core` ← `runtime-*` crates (并列) ← `runtime` facade ← `server`

**禁止**：
- `runtime-execution` 依赖 `runtime-session`
- `runtime-session` 依赖 `runtime-agent-loop` 或 `runtime-agent-control`
- `runtime` facade 包含业务逻辑或第二套实现层

## Five Boundaries (单一职责)

| Boundary | 职责 | 入口 |
|----------|------|------|
| `runtime-session` | Session 真相、durable replay | `sessions()` |
| `runtime-execution` | 执行编排、submit/interrupt、subrun status/cancel | `execution()` |
| `runtime-agent-loop` | 单次 LLM/tool 循环 | `TurnRunner` trait |
| `runtime-agent-control` | Live subrun registry、cancel cascade | `LiveSubRunControl` trait |
| `runtime` facade | 组装、注入、生命周期 | `RuntimeService` |

## 真相来源 (不可混淆)

- **Durable history** = 已完成 subrun 的唯一真相
- **Live state** = 仅补充运行中的状态
- **Session truth** 在 `runtime-session`，**execution orchestration** 在 `runtime-execution`，不得重叠
