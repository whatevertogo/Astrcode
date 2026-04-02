# ADR-0004: Freeze Layered Core, Runtime, and Transport Boundaries

- Status: Accepted
- Date: 2026-03-30
- Amended: 2026-04-03

## Context

AstrCode 当前的主要问题已经不再是"能力不够"，而是部分装配代码和核心语义开始混杂：

- `AgentLoop`、capability router、prompt contributors 已经初步形成稳定内核
- runtime 装配、plugin lifecycle、health / reload 逻辑分散在 `server` 与其他模块之间
- transport 层仍然容易承载过多业务装配职责

如果不及时冻结分层边界，后续继续接入 approval、skills / agents layering、ACP / CLI / MCP、多 provider / 多 capability source 时，核心语义和 transport / runtime 细节会继续互相污染。

## Decision

冻结 AstrCode 的高层分层为三层：

### 1. Layer 1: Immutable Core Contracts

Layer 1 只保留以下平台核心契约：

| 契约 | 源码路径 | 核心类型/方法 |
|------|---------|--------------|
| AgentLoop | `crates/runtime/src/agent_loop.rs` `crates/runtime/src/agent_loop/turn_runner.rs` | `AgentLoop`, `TurnOutcome` |
| Capability | `crates/core/src/capability.rs` `crates/core/src/registry.rs` | `CapabilityDescriptor`, `CapabilityRouter`, `CapabilityInvoker` |
| Policy | `crates/core/src/policy/mod.rs` | `PolicyEngine`, `PolicyVerdict<T>` (Allow/Deny/Ask) |
| Event | `crates/core/src/event/domain.rs` `crates/core/src/event/types.rs` | `AgentEvent`(SSE/观测), `StorageEvent`(持久化), `EventTranslator` |
| Tool | `crates/core/src/tool.rs` | `Tool`, `ToolContext`, `ToolDefinition`, `ToolExecutionResult` |
| Session | `crates/core/src/store.rs` | `SessionManager`, `EventLogWriter`, `SessionTurnLease` |

Layer 1 依赖：`protocol`(纯 DTO 层，无 workspace 依赖)。

Layer 1 不包含：PluginHost、具体 provider / tool / storage 实现、HTTP / SSE / Axum 细节、CLI / ACP / Tauri / Web 适配。

**关键路径映射**:
- `AgentEvent` (观测面): `crates/core/src/event/domain.rs` — 面向 UI/SSE/telemetry
- `StorageEvent` (持久化面): `crates/core/src/event/types.rs` — 面向 replay/cursor/session 恢复
- `EventTranslator`: `crates/core/src/event/translate.rs` — 做 `StorageEvent` → `AgentEvent` 投影
- `StoredEvent`: `crates/core/src/event/types.rs` — `storage_seq` + `event`, append-only JSONL 记录
- `Phase`: `crates/core/src/event/domain.rs` — `Idle | Thinking | CallingTool | Streaming | Interrupted | Done`

### 2. Layer 2: Runtime Assembly

Layer 2 负责把 core contract 组装为可运行 runtime：

| Crate | 源码路径 | 职责 | 核心类型/方法 |
|-------|---------|------|--------------|
| `runtime` | `crates/runtime/src/lib.rs` | 门面：`RuntimeService`、`AgentLoop`、bootstrap、governance | `RuntimeService`, `AgentLoop`, `RuntimeGovernance` |
| `storage` | `crates/storage/src/lib.rs` | JSONL 会话持久化 | `FileSystemSessionRepository` |
| `tools` | `crates/tools/src/lib.rs` | 内置工具(7个) | `read_file`, `write_file`, `edit_file`, `list_dir`, `find_files`, `grep`, `shell` |
| `runtime-config` | `crates/runtime-config/src/lib.rs` | 配置加载/校验/env 解析 | `Config`, `Profile`, `load_config`, `test_connection`, `resolve_api_key` |
| `runtime-llm` | `crates/runtime-llm/src/lib.rs` | LLM 提供者抽象 | `LlmProvider`(trait), `LlmRequest`, `LlmOutput`, `LlmEvent`, anthropic + openai |
| `runtime-prompt` | `crates/runtime-prompt/src/lib.rs` | Prompt Contributor 模式 | `PromptComposer`, `PromptContributor`, `SkillSummaryContributor` |
| `plugin` | `crates/plugin/src/lib.rs` | 插件宿主 | `Supervisor`, `Peer`, `PluginProcess`, `PluginCapabilityInvoker` |

**Runtime AgentLoop 子模块**:
- `crates/runtime/src/agent_loop/turn_runner.rs` — Turn 编排主循环
- `crates/runtime/src/agent_loop/llm_cycle.rs` — LLM 调用周期
- `crates/runtime/src/agent_loop/tool_cycle.rs` — 工具执行周期(含并行)
- `crates/runtime/src/agent_loop/compaction.rs` — 上下文压缩
- `crates/runtime/src/agent_loop/microcompact.rs` — 微调压缩
- `crates/runtime/src/agent_loop/token_budget.rs` — Token 预算控制
- `crates/runtime/src/agent_loop/token_usage.rs` — Token 用量统计

**Runtime Service 子模块**:
- `crates/runtime/src/service/turn_ops.rs` — Turn 执行操作
- `crates/runtime/src/service/session_ops.rs` — 会话 CRUD
- `crates/runtime/src/service/config_ops.rs` — 配置操作
- `crates/runtime/src/service/replay.rs` — 会话回放
- `crates/runtime/src/service/session_state.rs` — 会话状态管理
- `crates/runtime/src/service/observability.rs` — 可观测性指标

**关键装配流程** (`crates/runtime/src/bootstrap.rs`):
```
bootstrap_runtime()
  → RuntimeSurfaceAssembler::assemble()
    → LLM provider factory
    → CapabilityRouter (built-in tools + plugins)
    → PolicyEngine + ApprovalBroker
    → SessionManager (FileSystemSessionRepository)
  → RuntimeBootstrap { service, coordinator, governance }
```

### 3. Layer 3: Transports and External Adapters

Layer 3 负责对外暴露 runtime：

| 模块 | 源码路径 | 职责 |
|------|---------|------|
| `server` | `crates/server/src/main.rs` | HTTP/SSE API (Axum), 静态资源托管 |
| `src-tauri` | `src-tauri/` | Tauri 桌面壳, window 控制, sidecar 管理 |
| `frontend` | `frontend/src/` | React 前端 SPA |

**Server 路由**:
- `crates/server/src/routes/sessions.rs` — 会话 CRUD, turn 执行, SSE 事件流
- `crates/server/src/routes/config.rs` — 配置管理
- `crates/server/src/routes/model.rs` — 模型列表和选择
- `crates/server/src/routes/runtime.rs` — 运行时重载, 可观测性
- `crates/server/src/bootstrap.rs` — CORS, 前端构建加载, `run.json` 写入

**Server → Runtime 数据流**:
```
HTTP Client → Axum Router → routes/{sessions,config,model,runtime}
  → RuntimeService (via AppState)
    → SessionOps / TurnOps / ConfigOps
  → EventTranslator: StorageEvent → AgentEvent
  → SSE: /api/sessions/:id/events
```

## Consequences

正面影响：
- 后续功能演进优先发生在 runtime assembly 或 transport 层
- core 契约可以更稳定
- transport 不再天然拥有业务装配权
- PluginHost 不再被误视为核心语义

代价：
- 当前部分位于 `server` 的装配代码需要下沉或拆分
- runtime 层会显式承担更多框架化职责
- 新模块边界需要补充测试和文档以维持清晰度

## Current Implementation Status (2026-04-03)

- Layer 1: `crates/protocol` (纯 DTO) + `crates/core` (核心契约) — 编译隔离 ✓
- Layer 2: 7 个独立 crate, 编译隔离 ✓
- Layer 3: `crates/server` (HTTP/SSE) + `src-tauri` (桌面壳) + `frontend` (React SPA) ✓
