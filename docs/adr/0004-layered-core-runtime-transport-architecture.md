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

Layer 1 仅包含平台核心契约，位于 `protocol`（纯 DTO）和 `core`（行为契约）。

| 契约 | 源码路径 | 核心类型 |
|------|---------|----------|
| AgentLoop | `crates/runtime-agent-loop/src/agent_loop.rs` | `AgentLoop`, `TurnOutcome` |
| Capability | `crates/core/src/capability.rs` + `registry/` | `CapabilityDescriptor`, `CapabilityRouter`, `CapabilityInvoker` |
| Policy | `crates/core/src/policy/` | `PolicyEngine`, `PolicyVerdict<T>` (Allow/Deny/Ask) |
| Event | `crates/core/src/event/` | `AgentEvent`, `StorageEvent`, `EventTranslator` |
| Tool | `crates/core/src/tool.rs` | `Tool`, `ToolContext`, `ToolDefinition`, `ToolExecutionResult` |
| Session | `crates/core/src/store.rs` | `SessionManager`, `EventLogWriter`, `SessionTurnLease` |

**不包含**：PluginHost、Provider/Tool/Storage 具体实现、HTTP/SSE/Axum、CLI/Tauri/Web 适配。

**事件面划分**：
- `AgentEvent`（观测面）: `crates/core/src/event/domain.rs` — 面向 UI/SSE/telemetry
- `StorageEvent`（持久化面）: `crates/core/src/event/types.rs` — 面向 replay/cursor/session 恢复
- `EventTranslator`: `crates/core/src/event/translate.rs` — `StorageEvent` → `AgentEvent` 投影
- `Phase`: `crates/core/src/event/domain.rs` — `Idle | Thinking | CallingTool | Streaming | Interrupted | Done`

### 2. Layer 2: Runtime Assembly

Layer 2 负责将 Layer 1 契约组装为可运行的 runtime：

| Crate | 职责 | 核心模块 |
|-------|------|---------|
| `runtime` | 纯门面：`RuntimeService` + bootstrap，re-export 子 crate | `bootstrap.rs`, `runtime_governance.rs`, `service/` |
| `runtime-agent-loop` | AgentLoop 执行引擎，四层运行时 | `agent_loop/`, `prompt_runtime.rs`, `context_pipeline.rs`, `compaction_runtime.rs`, `request_assembler.rs` |
| `runtime-config` | 配置加载/校验/env 解析 | `types.rs`, `loader.rs`, `saver.rs`, `validation.rs` |
| `runtime-llm` | LLM 提供者抽象（anthropic + openai） | `lib.rs`, `provider.rs` |
| `runtime-prompt` | Prompt Contributor 模式 | `composer.rs`, `contributors/` |
| `runtime-skill-loader` | Skill 资源发现与 `SkillCatalog` | `skill_catalog.rs`, `skill_loader.rs` |
| `storage` | JSONL 会话持久化 | `session/event_log.rs`, `session/repository.rs` |
| `tools` | 内置工具 (7 个) | `tools/` (read_file, write_file, edit_file, list_dir, find_files, grep, shell) |
| `plugin` | 插件宿主 | `supervisor.rs`, `peer.rs` |

**关键装配流程** (`crates/runtime/src/bootstrap.rs`):
```
bootstrap_runtime()
  → RuntimeSurfaceAssembler::assemble()
    → LLM provider factory
    → CapabilityRouter (built-in tools + plugins)
    → PolicyEngine + ApprovalBroker (agent_loop)
    → SessionManager (FileSystemSessionRepository)
  → RuntimeBootstrap { service, governance }
```

### 3. Layer 3: Transports

| 模块 | 职责 |
|------|------|
| `server` | HTTP/SSE API (Axum)，静态资源托管 |
| `src-tauri` | Tauri 桌面壳，window 控制，sidecar 管理 |
| `frontend` | React 前端 SPA (详见 [frontend-architecture.md](../architecture/frontend-architecture.md)) |

**Server 路由**:
- `routes/sessions.rs` — 会话 CRUD, turn 执行, SSE 事件流
- `routes/config.rs` — 配置管理
- `routes/model.rs` — 模型列表和选择
- `routes/runtime.rs` — 运行时重载, 可观测性
- `routes/composer.rs` — Prompt composer 选项查询

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
- Layer 2: 7 个独立 runtime crate + `runtime-agent-loop` (执行引擎) — 编译隔离 ✓
- Layer 3: `crates/server` (HTTP/SSE) + `src-tauri` (桌面壳) + `frontend` (React SPA) ✓
