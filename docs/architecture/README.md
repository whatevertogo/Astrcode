# AstrCode Architecture Design

这些文档不是新的 ADR，而是对现有已接受 ADR 的补充说明，用于指导后续重构与模块拆分。

当前已接受且仍然生效的 ADR：

- [ADR-0001: Freeze AstrCode Protocol V4 Wire Model](../adr/0001-astrcode-protocol-v4-wire-model.md)
- [ADR-0002: Freeze Coding Profile Context Boundary](../adr/0002-astrcode-coding-profile-context.md)
- [ADR-0003: Use Core-Owned Unified Capability Routing](../adr/0003-unified-capability-routing.md)
- [ADR-0004: Freeze Layered Core, Runtime, and Transport Boundaries](../adr/0004-layered-core-runtime-transport-architecture.md)
- [ADR-0005: Split Policy Decision Plane from Event Observation Plane](../adr/0005-split-policy-decision-plane-from-event-observation-plane.md)

这组文档解决的问题不是“协议该不该改”，而是：

- AgentLoop、Capability、Policy、Event 四类核心契约如何进一步冻结
- runtime、plugin、transport 的职责如何重新拉开
- 当前代码中哪些结构值得保留，哪些需要整理性重构
- 后续的 skills、agents、approval、ACP/MCP 接入应该挂在哪一层

建议阅读顺序：

1. [01-layered-architecture.md](./01-layered-architecture.md)
2. [02-core-contracts.md](./02-core-contracts.md)
3. [03-runtime-assembly.md](./03-runtime-assembly.md)
4. [04-events-approval-and-transports.md](./04-events-approval-and-transports.md)
5. [05-refactor-roadmap.md](./05-refactor-roadmap.md)

## Design Summary

AstrCode 的目标架构收敛为三层：

- Layer 1：不可变核心，只保留执行语义和平台契约
- Layer 2：运行时装配层，负责把内置能力、外部插件、策略和持久化组装起来
- Layer 3：外部接入层，负责 HTTP/SSE、Tauri/Web/CLI/ACP 等 transport 与客户端适配

核心固定的不是某个具体 UI，也不是某个具体插件机制，而是四类平台契约：

- AgentLoop Contract
- Capability Contract
- Policy Contract
- Event Contract

## Scope

这些文档聚焦以下问题：

- 如何在不推翻现有 `AgentLoop` 的前提下整理架构
- 如何让 `Capability` 成为唯一一等动作模型
- 如何把同步决策与异步观测分离
- 如何把 `server is truth` 保留为产品架构原则，但不让 `server` 变成业务内核

这些文档不解决以下问题：

- 某个具体 provider 的实现细节
- 某个具体 tool 的参数设计
- 某个具体 UI 的交互稿
- MCP / ACP 的完整协议映射细节

## Current Code Anchors

当前仓库里与这组文档最相关的代码锚点如下：

- `crates/runtime/src/agent_loop.rs`
- `crates/runtime/src/agent_loop/turn_runner.rs`
- `crates/runtime/src/agent_loop/tool_cycle.rs`
- `crates/runtime/src/bootstrap.rs`
- `crates/runtime/src/runtime_surface_assembler.rs`
- `crates/runtime/src/runtime_governance.rs`
- `crates/core/src/registry/router.rs`
- `crates/core/src/capability.rs`
- `crates/server/src/main.rs`
- `crates/plugin/src/*`

后续重构时，优先以这些文件为迁移入口，而不是从 UI 或单个 tool 开始逆向调整。
