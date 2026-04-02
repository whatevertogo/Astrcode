# AstrCode Architecture

## Crate Dependency Graph

```text
protocol (纯 DTO，零业务依赖)
    ↑
core (核心契约：Event/Policy/Capability/Tool trait + 持久化接口)
    ↑
storage (JSONL 会话持久化实现)
tools (内置工具)    runtime-config (配置)    runtime-llm (LLM)    runtime-prompt (Prompt)    plugin (插件宿主)
    ↑                               ↑           ↑           ↑               ↑
    +────────────────── runtime (RuntimeService 门面 + AgentLoop) ──────────+
                                       ↑
                                    server (HTTP/SSE)
```

实际依赖关系（workspace 内）：

| Crate | 依赖的 workspace crate |
|-------|----------------------|
| `protocol` | 无（叶子节点） |
| `core` | `protocol` |
| `storage` | `core` |
| `tools` | `core` |
| `runtime-config` | `core` |
| `runtime-llm` | `core` |
| `runtime-prompt` | `core` |
| `plugin` | `core` + `protocol` |
| `runtime` | 以上全部 |
| `server` | `core` + `protocol` + `runtime` |
| `sdk` | `protocol`（独立客户端 SDK） |

---

## Three-Layer Architecture

### Layer 1: Immutable Core Contracts

`crates/core` + `crates/protocol`。只放"平台事实"，不放"产品选择"。

| 契约 | 位置 | 职责 |
|------|------|------|
| AgentLoop | `runtime/src/agent_loop.rs` | 唯一执行语义 |
| Capability | `core/src/capability.rs` | 唯一动作模型 |
| Policy | `core/src/policy/engine.rs` | 唯一同步决策面 |
| Event | `core/src/event/` | 唯一异步观测面 |

**不进入 Layer 1 的东西**：PluginHost、具体 Provider 实现、文件系统工具、SessionStore 后端、HTTP/SSE、Tauri、CLI。

### Layer 2: Runtime Assembly

把 core 契约组装成可运行 runtime：

| Crate | 职责 |
|-------|------|
| `storage` | JSONL 会话持久化（`EventLog`、`FileSystemSessionRepository`） |
| `tools` | 内置工具（fs、shell 等），通过 `ToolCapabilityInvoker` 注册 |
| `runtime-config` | 配置模型与加载/校验 |
| `runtime-llm` | LLM 提供者抽象与 OpenAI/Anthropic 适配 |
| `runtime-prompt` | Prompt Contributor 模式（Identity/AgentsMd/Environment/Skill 索引） |
| `plugin` | 插件宿主（supervisor、peer、transport） |
| `runtime` | 门面：`AgentLoop` + `RuntimeService` + bootstrap + governance |

关键入口：
- `runtime/src/bootstrap.rs` — 产出 `RuntimeSurface`
- `runtime/src/runtime_surface_assembler.rs` — 统一 capability surface 装配
- `runtime/src/runtime_governance.rs` — reload / health / snapshot
- `runtime/src/agent_loop/` — `turn_runner.rs`、`tool_cycle.rs`、`llm_cycle.rs`

Skill 相关的实现说明见：
- [skills-architecture.md](./skills-architecture.md)

### Layer 3: Transports

`crates/server` + `src-tauri` + `frontend`。对外暴露 runtime，不定义 agent 语义。

`server is truth` 是产品架构原则，但 `server` 在代码分层上属于 transport 层。

---

## Four Core Contracts

### 1. AgentLoop Contract

Turn 是基本调度单位。AgentLoop 按 turn 调度，Policy 按 turn 决策，Event 按 turn 关联。

```text
loop {
    build_request → policy.check_model_request → call_llm
    for each capability_call:
        policy.check_capability_call → Allow / Deny / Ask
        Ask → ApprovalBroker.resolve → Allow / Deny
    check context pressure → compact if needed
} until stop_reason == EndTurn or cancel
```

### 2. Capability Contract

Capability 是唯一一等动作模型。`CapabilityKind` 是路由元数据，不是第二套协议。

```text
Tool → ToolCapabilityInvoker → CapabilityRouter ← PluginCapabilityInvoker ← Plugin
                                     ↑
                              runtime 只消费 router
```

`CapabilityDescriptor` 校验在装配阶段统一执行，不依赖 builder。

### 3. Policy Contract

Policy 拥有改变执行结果的权力。三态决策：

```rust
enum PolicyVerdict<T> {
    Allow(T),
    Deny { reason: String },
    Ask(ApprovalPending<T>),
}
```

三个决策点：`check_model_request`、`check_capability_call`、`decide_context_strategy`。

`Ask` 分支通过 `ApprovalBroker` 挂起/恢复，不通过 EventBus。

### 4. Event Contract

Event 只表达"发生了什么"，不表达"下一步该怎么做"。

两类事件，通过 `EventTranslator` 互相投影，不强制等同：

| 类型 | 位置 | 用途 |
|------|------|------|
| `AgentEvent` | `core/src/event/domain.rs` | 运行时观测：UI/SSE/telemetry |
| `StorageEvent` | `core/src/event/types.rs` | 持久化：replay/cursor/session 恢复 |

持久化实现在 `storage` crate：`EventLog`（append-only JSONL）、`FileSystemSessionRepository`（会话管理）。

---

## Key Design Rules

1. **`protocol` 不得依赖 `core`/`runtime`**；跨边界数据走显式 DTO + mapper
2. **`core` 不持有运行态**；进程内运行态（broadcast、cancel、活动 session）放在 `runtime`
3. **Transport 不定义 agent 语义**；HTTP/SSE/Tauri 只消费 runtime surface
4. **Capability 是唯一动作模型**；不为 tool/workflow/plugin 维护独立调用协议
5. **Policy 是唯一同步决策面**；Event 只负责观测
6. **持久化实现与核心契约分离**；`core` 定义接口（`EventLogWriter`、`SessionManager`），`storage` 提供文件系统实现
7. **`tools` 仅依赖 `core`**，不直接依赖 `runtime`
