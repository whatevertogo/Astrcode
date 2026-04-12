# 模块迁移对照（project-architecture-refactor）

本文档记录本轮架构重构中“旧模块 -> 新模块”的最终落点，便于后续排障与代码检索。

## 1. 运行时主干迁移

- `crates/runtime` -> 删除，职责拆分到 `kernel` / `session-runtime` / `application` / `server`
- `runtime/service/session/*` -> `crates/session-runtime/src/catalog` 与 `crates/application/src/lib.rs`（用例入口）
- `runtime/service/turn/*` -> `crates/session-runtime/src/turn`
- `runtime/service/agent/*` -> `crates/session-runtime/src/observe`（状态与过滤模型）+ `session-runtime/state`
- `runtime/service/config/*` -> `crates/application/src/config`
- `runtime/service/composer/*` -> `crates/application/src/composer`
- `runtime/service/mcp/*` -> `crates/application/src/mcp`
- `runtime/service/lifecycle/*` / `service_contract.rs` -> `crates/application/src/lifecycle` + `crates/application/src/errors.rs`
- `runtime/service/observability/*` -> `crates/application/src/observability`

## 2. 旧 runtime 子 crate 迁移

- `runtime-config` -> 稳定模型迁入 `core/config`，IO 与校验迁入 `application/config`
- `runtime-registry` -> `kernel/registry`
- `runtime-agent-control` -> `kernel/agent_tree`
- `runtime-agent-loop` -> `session-runtime/turn` + `session-runtime/factory`
- `runtime-session` -> `session-runtime/state`
- `runtime-execution` -> `session-runtime/actor` + `session-runtime/context`

## 3. 实现层命名收敛（adapter-*）

- `storage` -> `adapter-storage`
- `runtime-llm` -> `adapter-llm`
- `runtime-prompt` -> `adapter-prompt`
- `runtime-mcp` -> `adapter-mcp`
- `runtime-tool-loader + runtime-agent-tool` -> `adapter-tools`
- `runtime-skill-loader` -> `adapter-skills`
- `runtime-agent-loader` -> `adapter-agents`

## 4. 边界约定

- `server` handler 仅依赖 `application`（`App` / `AppGovernance`）。
- `application` 仅依赖 `core + kernel + session-runtime`。
- `adapter-*` 仅依赖 `core` 与第三方库，不反向依赖上层。
- 运行时内部统一使用 `CapabilitySpec`，`CapabilityDescriptor` 仅保留协议边界 DTO 角色。
