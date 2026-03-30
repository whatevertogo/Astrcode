# Layered Architecture

## Goal

AstrCode 未来要稳定的不是“某个 server 实现”或“某个插件加载器实现”，而是平台最小执行语义与跨层契约。

因此架构分为三层：

```text
┌──────────────────────────────────────────────┐
│ Layer 3: Transports and External Adapters    │
│ HTTP/SSE, Tauri/Web, CLI, ACP, MCP bridge    │
├──────────────────────────────────────────────┤
│ Layer 2: Runtime Assembly                    │
│ PluginHost, Router, Policy, Approval, Store  │
├──────────────────────────────────────────────┤
│ Layer 1: Immutable Core Contracts            │
│ AgentLoop, Capability, Policy, Event         │
└──────────────────────────────────────────────┘
```

## Layer 1: Immutable Core Contracts

这一层只放“平台事实”，不放“产品选择”。

应保留在 Layer 1 的东西：

- `AgentLoop` 的执行语义
- `Capability` 的统一动作模型
- `Policy` 的同步决策模型
- `Event` 的异步观测模型
- 与上面四者配套的基础类型，例如 `CapabilityCall`、`CapabilityResult`、`StopReason`

不应该进入 Layer 1 的东西：

- `PluginHost`
- 具体 `Provider` 实现
- 文件工具、shell 工具、MCP bridge
- `SessionStore` 的某个具体后端
- compaction 的某个具体算法
- HTTP/SSE、Axum、Tauri、CLI

## Layer 2: Runtime Assembly

这一层是真正“把系统跑起来”的地方。

职责包括：

- 装配内置 capability 与外部 capability
- 挂接 `PolicyEngine`
- 管理 `ApprovalBroker`
- 发布 `AgentEvent`
- 维护 `SessionStore`
- 发现和加载 `AGENTS.md` / `SKILL.md` / layered config
- 启停、reload、health check、crash recovery

可以替换，但不应该影响 Layer 1 契约。

## Layer 3: Transports and External Adapters

这一层的职责是把 runtime 暴露给不同客户端或外部生态，而不是定义 agent 语义。

包括：

- HTTP API
- SSE event stream
- Tauri sidecar bootstrap
- Web frontend
- CLI viewer/controller
- ACP server
- MCP external plugin bridge

`server is truth` 是 AstrCode 的产品架构原则，但代码分层上 `server` 仍然属于 Layer 3。  
它可以是平台唯一业务入口，但不应该成为平台核心语义的定义者。

## Why Server Is Not Core

当前仓库已经体现出这一点：

- `server` 负责启动、认证、CORS、HTTP/SSE 和 runtime 装配
- `runtime` 负责执行 `AgentLoop`
- `core` 负责 `CapabilityRouter`、descriptor 和共享类型

相关代码：

- `crates/server/src/main.rs`
- `crates/runtime/src/agent_loop.rs`
- `crates/core/src/registry/router.rs`

这条边界应该继续强化，而不是回退。

## Why PluginHost Is Not Core

`PluginHost` 解决的是运行时装配问题，而不是执行语义问题。

它负责：

- discovery
- init
- lifecycle
- health
- reload
- supervisor / crash isolation

这些都是 Layer 2 的事情。

把 `PluginHost` 放进 core 会导致：

- test host 和 prod host 无法自然替换
- core 被进程管理和生命周期细节污染
- 将来如果换一套静态装配方式，core 也要跟着变

## Mapping to Current Repo

当前仓库和目标分层的对应关系大致如下：

| 目标层 | 当前 crate / 模块 | 说明 |
| --- | --- | --- |
| Layer 1 | `crates/runtime/src/agent_loop*`, `crates/core/src/capability.rs`, `crates/core/src/registry/router.rs` | 已经有核心雏形，但命名和边界还需整理 |
| Layer 2 | `crates/server/src/capabilities/*`, `crates/plugin/src/*`, `crates/runtime/src/service/*`, prompt contributors | 装配逻辑已经从单文件拆成模块，但仍主要位于 `server` 侧，后续还要继续收敛 runtime surface |
| Layer 3 | `crates/server/src/main.rs`, `crates/server/src/routes/*`, `src-tauri/src/main.rs`, `frontend/src/hooks/useAgent.ts` | 已经基本按 transport 层工作 |

## Design Consequences

好处：

- 底层契约一旦冻结，功能扩展主要在 Layer 2 和 Layer 3 发生
- UI、transport 和插件实现可以持续替换
- provider/tool/storage/workflow 不再需要各自发明一套动作协议

代价：

- Layer 2 会比现在更明确，也会更“像一层框架”
- 一些当前放在 `server` 的装配代码需要下沉或拆分
- `ToolRegistry` 需要退化为 capability source，而不是继续和 capability 并列
