# AstrCode Architecture

## Crate Dependency Graph

```text
protocol (纯 DTO，零业务依赖)
    ↑
core (核心契约：Event/Policy/Capability/Tool trait + 持久化接口)
    ↑
storage (JSONL 会话持久化实现)
tools (内置工具)    runtime-config (配置)    runtime-llm (LLM)    runtime-prompt (Prompt)    runtime-skill-loader (Skill)    plugin (插件宿主)    sdk (插件 SDK)
    ↑                     ↑                       ↑                     ↑                          ↑                          ↑                    ↑
    +────────────── runtime-agent-loop (AgentLoop 执行引擎) ──────────────────────────────────────────────────────+
                                        ↑
                          runtime (RuntimeService 门面，re-export 子 crate)
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
| `runtime-skill-loader` | `core` |
| `runtime-agent-loop` | `core` + `runtime-llm` + `runtime-prompt` + `runtime-skill-loader` + `runtime-config` |
| `plugin` | `core` + `protocol` |
| `sdk` | `protocol` |
| `runtime` | `runtime-agent-loop` + `runtime-config` + `runtime-llm` + `runtime-prompt` + `runtime-skill-loader`（纯 re-export 门面） |
| `server` | `core` + `protocol` + `runtime` |

---

## Three-Layer Architecture

### Layer 1: Immutable Core Contracts

`crates/protocol` + `crates/core`。只放"平台事实"，不放"产品选择"。

| 模块 | 位置 | 核心类型 | 职责 |
|------|------|---------|------|
| DTO 协议 | `crates/protocol/src/` | `http::*`, `plugin::*`, `transport::*` | 跨模块通信协议 |
| AgentLoop | `crates/runtime-agent-loop/src/agent_loop.rs` | `AgentLoop`, `TurnOutcome` | 唯一执行语义 |
| Capability | `crates/core/src/capability.rs` `crates/core/src/registry/` | `CapabilityDescriptor`, `CapabilityKind`, `CapabilityRouter`, `CapabilityInvoker` | 唯一动作模型 |
| Policy | `crates/core/src/policy/` | `PolicyEngine`, `PolicyVerdict<T>` (Allow/Deny/Ask) | 唯一同步决策面 |
| Event | `crates/core/src/event/` | `AgentEvent`(观测), `StorageEvent`(持久化), `Phase`, `EventTranslator` | 唯一异步观测面 |
| Tool | `crates/core/src/tool.rs` + `action.rs` | `Tool` trait, `ToolContext`, `ToolDefinition`, `ToolExecutionResult` | 工具抽象接口 |
| Session Store | `crates/core/src/store.rs` | `SessionManager`, `EventLogWriter`, `SessionTurnLease` | 持久化接口 |

**不进入 Layer 1 的东西**：PluginHost、具体 Provider 实现、文件系统工具、SessionStore 后端、HTTP/SSE、Tauri、CLI。

### Layer 2: Runtime Assembly

把 core 契约组装成可运行 runtime：

| Crate | 核心入口 | 职责 |
|-------|---------|------|
| `runtime` | `crates/runtime/src/lib.rs` | 纯门面：`RuntimeService`，re-export 子 crate |
| `runtime-agent-loop` | `crates/runtime-agent-loop/src/lib.rs` | `AgentLoop`, `TurnOutcome`, prompt/context/compaction/assembler 四层运行时 |
| `runtime-config` | `crates/runtime-config/src/` | 配置模型与加载/校验/env 解析 |
| `runtime-llm` | `crates/runtime-llm/src/` | LLM 提供者抽象，OpenAI-compatible/Anthropic 适配 |
| `runtime-prompt` | `crates/runtime-prompt/src/` | Prompt Contributor 模式、`PromptComposer`、`LayeredPromptBuilder` |
| `runtime-skill-loader` | `crates/runtime-skill-loader/src/` | Skill 资源发现、解析、目录扫描、`SkillCatalog` |
| `storage` | `crates/storage/src/session/` | JSONL 会话持久化（`FileSystemSessionRepository`） |
| `tools` | `crates/tools/src/tools/` | 内置工具(7个)：`readFile`, `writeFile`, `editFile`, `listDir`, `findFiles`, `grep`, `shell` |
| `plugin` | `crates/plugin/src/` | 插件宿主（`Supervisor`, `Peer`, `PluginCapabilityInvoker`） |
| `sdk` | `crates/sdk/src/` | 插件开发 SDK（protocol 兼容的流式、错误处理、工具上下文） |

**关键装配流程** (`crates/runtime/src/bootstrap.rs`):
`RuntimeBootstrap { service: Arc<RuntimeService>, coordinator: Arc<RuntimeCoordinator>, governance: Arc<RuntimeGovernance> }`

`runtime` crate 是纯门面，通过 `pub use` re-export 所有子 crate：
- `astrcode_runtime_agent_loop as agent_loop`
- `astrcode_runtime_config as config`
- `astrcode_runtime_llm as llm`
- `astrcode_runtime_prompt as prompt`
- `astrcode_runtime_skill_loader as skills`

**Runtime Service 内部子模块** (`crates/runtime/src/service/`):
- `turn_ops.rs` — turn 执行、事件广播、EventTranslator 投影
- `session_ops.rs` — 会话 CRUD (create/delete/fork/list)
- `config_ops.rs` — 配置查询/更新
- `composer_ops.rs` — composer 选项管理
- `replay.rs` — 会话事件回放 (SSE 断线重连)
- `session_state.rs` — 每会话状态 (AgentState + 事件日志)
- `baselines.rs` — Session 基线快照
- `support.rs` — 辅助/诊断工具
- `observability.rs` — 运行时指标快照

**AgentLoop 内部子模块** (`crates/runtime-agent-loop/src/`):

核心运行时（四层分离，详见 [ADR-0008](../adr/0008-agent-loop-content-architecture.md)）：
- `prompt_runtime.rs` — Prompt 运行时：桥接 `PromptComposer`，只负责"说明书"
- `context_pipeline.rs` — Context 运行时：Pipeline + Stage 模式，只负责"材料选择"
- `compaction_runtime.rs` — Compaction 运行时：Policy + Strategy + Rebuilder 三件套
- `request_assembler.rs` — 请求装配：`PromptPlan + ContextBundle + Tools → ModelRequest` 唯一边界

执行层：
- `agent_loop.rs` — `AgentLoop` 结构体与 builder API
- `agent_loop/turn_runner.rs` — Turn 编排主循环 (状态机骨架)
- `agent_loop/llm_cycle.rs` — LLM provider 构建与调用
- `agent_loop/tool_cycle.rs` — 工具执行 (含并行执行)、Policy 三态检查、Approval Broker 集成
- `agent_loop/token_budget.rs` — Token 预算解析与续命决策

上下文窗口算法：
- `context_window/compaction.rs` — 上下文压缩算法
- `context_window/microcompact.rs` — 微调压缩
- `context_window/token_usage.rs` — Token 用量统计

**其他重要运行时模块** (`crates/runtime/src/`):
- `skill_tool.rs` — `Skill` tool 实现（按需加载 SKILL.md 正文）
- `builtin_capabilities.rs` — 内置能力装配
- `runtime_surface_assembler.rs` — 运行时表面装配
- `plugin_discovery.rs` — 插件发现
- `plugin_skill_materializer.rs` — 插件 skill 资源落盘
- `runtime_governance.rs` — 配置热重载、插件健康监控

**模型 limits 解析**:
- `runtime-config` 把 `Profile.models` 规范化为逐模型对象 `{ id, maxTokens?, contextLimit? }`
- OpenAI-compatible 模型的 `maxTokens/contextLimit` 只来自本地配置
- Anthropic 模型在 `runtime::provider_factory` 构造 provider 前调用 `GET /v1/models/{model_id}` 拉取 `max_input_tokens/max_tokens`，本地值仅作失败兜底
- `runtime-llm` provider 内部只消费已经解析好的 `ModelLimits`，不再各自硬编码 128k / 200k

### Layer 3: Transports

`crates/server` + `src-tauri` + `frontend`。对外暴露 runtime，不定义 agent 语义。

**Server 路由** (`crates/server/src/routes/`):
| 路由模块 | 端点 | 职责 |
|---------|------|------|
| `sessions.rs` | `POST /api/sessions`, `GET /api/sessions`, `DELETE /api/sessions/:id`, `POST /api/sessions/:id/prompts`, `GET /api/sessions/:id/messages`, `GET /api/sessions/:id/events`, `POST /api/sessions/:id/interrupt`, `POST /api/sessions/:id/compact`, `DELETE /api/projects` | 会话 CRUD、turn 执行、SSE 事件流 |
| `config.rs` | `GET /api/config`, `POST /api/config/active-selection` | 配置查询/更新 |
| `model.rs` | `GET /api/models`, `GET /api/models/current`, `POST /api/models/test` | 模型列表/连接测试/当前模型 |
| `runtime.rs` | `GET /api/runtime/plugins`, `POST /api/runtime/plugins/reload` | 运行时插件状态/重载 |
| (catalog) | `GET /api/session-events` | 全局会话目录 SSE（创建/删除/分支） |

**Server 认证** (`crates/server/src/auth.rs` + `bootstrap.rs`):
- `BootstrapAuth` — 短期 bootstrap token (24h TTL), 常量时间比较
- `AuthSessionManager` — 长期会话 token (8h TTL), 自动清理过期
- `AUTH_HEADER_NAME = "x-astrcode-token"` — 支持 header 和 query param

**Tauri 桌面壳** (`src-tauri/`):
- 窗口控制
- Sidecar 管理（`astrcode-server` 复制到 `~/.astrcode/runtime/sidecars/` 后启动）
- 多实例复用（通过 `~/.astrcode/run.json` 发现现有 server）

**前端** (`frontend/src/`):
- React 18 + TypeScript + Vite 单页应用
- 状态管理：`useReducer` + `store/reducer.ts`
- SSE 双通道：Agent events (`/api/sessions/:id/events`) + Session catalog events (`/api/session-events`)
- 详见 [frontend-architecture.md](./frontend-architecture.md)

---

## Four Core Contracts

### 1. AgentLoop Contract

Turn 是基本调度单位。AgentLoop 按 turn 调度，Policy 按 turn 决策，Event 按 turn 关联。

```
loop {
    PromptRuntime.build_plan → PromptPlan
    ContextRuntime.build_bundle → ContextBundle
    CompactionRuntime → maybe compact / rebuild conversation view
    RequestAssembler.assemble → ModelRequest
    policy.check_model_request → call_llm
    for each capability_call (并行执行):
        policy.check_capability_call → Allow / Deny / Ask
        Ask → ApprovalBroker.resolve → Allow / Deny
    CompactionPolicy → PolicyEngine.decide_context_strategy → compact if needed
    check token budget → continue with nudge or stop
} until LLM 返回纯文本 或 CancelToken 触发
```

执行结果通过 `TurnOutcome` 枚举显式表达：
```rust
pub enum TurnOutcome {
    Completed,   // LLM 返回纯文本（无 tool_calls），自然结束
    Cancelled,   // 用户取消或 CancelToken 触发
    Error { message: String },  // 不可恢复错误
}
```

详见 [ADR-0006](../adr/0006-turn-outcome-state-machine.md)。

### 2. Capability Contract

Capability 是唯一一等动作模型。`CapabilityKind` 是路由元数据，不是第二套协议。

```
Tool → ToolCapabilityInvoker → CapabilityRouter ← PluginCapabilityInvoker ← Plugin
                                     ↑
                              runtime 只消费 router
```

`CapabilityDescriptor` 校验在装配阶段统一执行，不依赖 builder。
`Tool` trait 提供 `capability_descriptor()` 默认实现，从 `definition()` + `capability_metadata()` 自动构建。

**`CapabilityKind` 变体**: `tool()`, `agent()`, `context_provider()`, `memory_provider()`, `policy_hook()`, `renderer()`, `resource()`, `prompt()`.

### 3. Policy Contract

Policy 拥有改变执行结果的权力。三态决策：

```rust
pub enum PolicyVerdict<T> {
    Allow(T),
    Deny { reason: String },
    Ask(Box<ApprovalPending<T>>),
}
```

三个决策点：`check_model_request`、`check_capability_call`、`decide_context_strategy`。

`Ask` 分支通过 `ApprovalBroker` 挂起/恢复，不通过 EventBus。

默认实现：`AllowAllPolicyEngine` 放行一切。

### 4. Event Contract

Event 只表达"发生了什么"，不表达"下一步该怎么做"。

两类事件，通过 `EventTranslator` 互相投影，不强制等同：

| 类型 | 位置 | 用途 | 消费者 |
|------|------|------|--------|
| `AgentEvent` | `core/src/event/domain.rs` | 运行时观测：UI/SSE/telemetry | 前端 SSE |
| `StorageEvent` | `core/src/event/types.rs` | 持久化：replay/cursor/session 恢复 | JSONL 持久化 |

`StorageEvent` 变体: `SessionStart`, `UserMessage`, `AssistantDelta`, `ThinkingDelta`, `AssistantFinal`, `ToolCall`, `ToolCallDelta`, `ToolResult`, `PromptMetrics`, `CompactApplied`, `TurnDone`, `Error`。

持久化实现在 `storage` crate：
- `EventLog` (append-only JSONL): `crates/storage/src/session/event_log.rs`
- `FileSystemSessionRepository` (会话管理): `crates/storage/src/session/repository.rs`

`StoredEvent { storage_seq, event: StorageEvent }` — `storage_seq` 由会话 writer 独占分配，`StoredEventLine` 向后兼容旧格式（无 `storage_seq`）。

SSE 事件 id 格式: `{storage_seq}.{subindex}`，客户端通过 `?afterEventId=` 参数断线重连。

**`Phase` 枚举**: `Idle`, `Thinking`, `CallingTool`, `Streaming`, `Interrupted`, `Done` — 通过 `PhaseTracker` 从 `StorageEvent` 流自动追踪。

---

## Key Design Rules

1. **`protocol` 不得依赖 `core`/`runtime`**；跨边界数据走显式 DTO + mapper
2. **`core` 不持有运行态**；进程内运行态（broadcast、cancel、活动 session）放在 `runtime`
3. **Transport 不定义 agent 语义**；HTTP/SSE/Tauri 只消费 runtime surface
4. **Capability 是唯一动作模型**；不为 tool/workflow/plugin 维护独立调用协议
5. **Policy 是唯一同步决策面**；Event 只负责观测
6. **持久化实现与核心契约分离**；`core` 定义接口（`EventLogWriter`, `SessionManager`），`storage` 提供文件系统实现
7. **`tools` 仅依赖 `core`**，不直接依赖 `runtime`
8. **`runtime-prompt`、`runtime-llm`、`runtime-config` 为独立 crate**，保持编译隔离
9. **Server is the Truth** — 所有会话、配置、模型、事件流业务入口只通过 `server` 暴露的 API

---

## Session / Event Model

- **全局配置**: `~/.astrcode/config.json`（项目级 overlay: `~/.astrcode/projects/<hash>.json`）
- **会话存储**: `~/.astrcode/projects/<project>/sessions/<session-id>/session-*.jsonl`
- **JSONL 格式**: append-only `StoredEvent { storage_seq, event: StorageEvent }`
- **会话 turn 锁**: 跨进程文件锁（`fs2`），`SessionTurnBusy` 返回占用者 PID
- **SSE 断线重连**: `GET /api/sessions/:id/events?afterEventId=` — 先通过 `SessionReplaySource` 回放历史，再实时订阅广播
- **Bootstrap 发现**: `~/.astrcode/run.json` (port, token, pid, expires_at_ms)

## Bootstrap 时序

```
main() → bootstrap_runtime() → RuntimeBootstrap { service, coordinator, governance }
  → 绑定 127.0.0.1:0 (随机端口)
  → 生成 bootstrap token (24h TTL)
  → 写入 ~/.astrcode/run.json (多实例发现)
  → 加载 frontend/dist/ (如存在) → 注入 window.__ASTRCODE_BOOTSTRAP__
  → 构建 Axum Router + CORS (允许 localhost:5173)
  → 启动 HTTP server + graceful shutdown
```

Vite 前端先启动 → Tauri sidecar 后启动 → 首个 API 请求等待 `window.__ASTRCODE_BOOTSTRAP__` 注入。

## 配置模型

```rust
pub struct Config {
    version: String,                 // 当前 "1"
    active_profile: String,          // 默认 "deepseek"
    active_model: String,            // 默认 "deepseek-chat"
    runtime: RuntimeConfig,          // 运行时配置
    profiles: Vec<Profile>,          // Provider profiles
}

pub struct RuntimeConfig {
    max_tool_concurrency: usize,         // 默认 10 (env: ASTRCODE_MAX_TOOL_CONCURRENCY)
    auto_compact_enabled: bool,          // 默认 true
    compact_threshold_percent: u8,       // 默认 90
    tool_result_max_bytes: usize,        // 默认 100_000
    compact_keep_recent_turns: u8,       // 默认 4
    default_token_budget: u64,           // 默认 0 (不限制)
    continuation_min_delta_tokens: u64,  // 默认 500
    max_continuations: usize,            // 默认 3
}
```

## 认证模型

两层 Token：

1. **Bootstrap Token** (24h TTL) — server 启动时生成，写入 `run.json`，用于首次握手的身份认证
2. **Session Token** (8h TTL) — 通过 `POST /api/auth/exchange` 用 bootstrap token 交换，后续所有 API 请求通过 `x-astrcode-token` header 注入

安全特性：常量时间比较 (`secure_token_eq`)、自动清理过期 session token。
