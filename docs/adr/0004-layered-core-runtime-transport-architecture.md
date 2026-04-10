# ADR-0004: Freeze Layered Core, Runtime, and Transport Boundaries

- Status: Accepted
- Date: 2026-03-30

## Context

AstrCode 的实现已经演进到多个子 crate：`core`、`runtime` 系列、`storage`、`server`、`src-tauri`、`frontend`、`plugin` 等。为了保持模块职责清晰，需要冻结三层边界，防止 transport 或实现细节反向污染核心契约。

## Decision

把系统划分成三层边界：核心契约层、运行时装配层、传输适配层。

- 核心契约层只包含稳定的 DTO 与行为契约，如 `Tool`、`PolicyEngine`、`AgentEvent`、`StorageEvent`、`ExecutionContext` 等。该层不包含具体 provider、插件宿主、存储实现或传输适配。
- 运行时装配层负责把核心契约组装成可运行系统，包括 `crates/runtime`、`crates/runtime-agent-loop`、`crates/runtime-llm`、`crates/runtime-prompt`、`crates/runtime-registry`、`crates/runtime-skill-loader`、`crates/runtime-tool-loader`、`crates/runtime-agent-control`、`crates/runtime-execution`、`crates/runtime-session` 等实现。
- 传输适配层只负责对外接入形式，如 `crates/server` 的 HTTP/SSE、`src-tauri` 的桌面壳，以及前端 UI；它不拥有核心业务语义，也不主导 runtime 装配。
- `storage` 属于持久化实现层，负责事件/会话记录与查询；它实现 core 定义的持久化接口。
- 核心契约的演进优先发生在 `crates/core` 或 `crates/protocol`；具体运行时变更优先落在 `runtime` 或 transport 层，而非让 transport 反向定义核心抽象。

## Consequences

- `core` 与 transport 之间的依赖方向更清晰，runtime 可以保持装配层职责。
- `server` / `src-tauri` 作为投影与接入层，不应承担业务执行逻辑。
- `storage` 提供持久化实现，但不会改变 `core` 的抽象契约。
- 该边界需要通过代码审查与依赖校验持续维护。
