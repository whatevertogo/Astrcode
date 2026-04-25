# Astrcode 项目架构

本文档描述仓库的**当前实际架构约束**。它不是历史说明，也不是愿景文档；实现与本文冲突时，优先修实现或修本文，不保留模糊兼容层。

## 架构原则

- 不维护向后兼容。旧边界阻碍正确架构时，迁移调用点并删除旧类型。
- `server` 是唯一组合根，负责装配、热替换和 HTTP/SSE 暴露。
- `core` 只放稳定、无副作用、无宿主策略的共享语义对象和 durable event 模型。
- contract crate 只定义窄领域契约，不放宿主 I/O、adapter 逻辑或运行时编排。
- owner crate 持有各自真相：
  - `host-session`：session durable truth、投影、fork、input queue、child session。
  - `plugin-host`：插件和外部贡献的 active snapshot、hook 注册与调度。
  - `agent-runtime`：单 turn 执行循环、hook dispatch port 消费。
  - `context-window`：上下文窗口、compact、请求整形。
- adapter 之间禁止横向依赖。跨 adapter 类型必须下沉到对应 contract 或 owner port。
- durable/live 分层必须清晰：JSONL durable event 是恢复、fork、历史回放的权威事实；SSE live event 只承担低延迟视图更新。
- 类型边界显式转换。不同层的 mode、prompt、tool、runtime event 不得靠 re-export 偷渡。

## 分层视图

```text
entry
  src-tauri       桌面薄壳（Tauri sidecar）
  cli             终端 TUI（ratatui）
  eval            评测 runner
  frontend        React UI

client
  astrcode-client HTTP/SSE SDK

composition
  astrcode-server 唯一组合根 + Axum API

adapters
  adapter-agents adapter-llm adapter-mcp adapter-prompt
  adapter-skills adapter-storage adapter-tools

owners / runtime
  host-session plugin-host agent-runtime context-window

contracts
  prompt-contract governance-contract tool-contract
  llm-contract runtime-contract

base
  core protocol support
```

## Crate 职责

### Base

| Crate | 职责 |
|---|---|
| `core` | 强类型 ID、`CapabilitySpec`、phase、durable storage event、agent 协作模型、错误类型、环境变量名常量 |
| `protocol` | HTTP/SSE/plugin wire DTO；`CapabilityWireDescriptor` 只用于传输边界 |
| `support` | host path、project dir name、shell 检测、大型 tool result 文件引用 |

### Contracts

| Crate | 职责 |
|---|---|
| `prompt-contract` | prompt declaration、prompt source/kind/render target、cache hint |
| `governance-contract` | mode DSL（`GovernanceModeSpec`）、tool policy、action policy、治理 prompt/model request |
| `tool-contract` | `Tool`、`ToolContext`、tool event sink、stream delta |
| `llm-contract` | `LlmProvider`、request/output、usage、model limits |
| `runtime-contract` | runtime handle、turn event、execution accepted、**hook typed payload/effect 合同**（`HookEventPayload`、`HookEffect`） |

### Owners / Runtime

| Crate | 职责 |
|---|---|
| `context-window` | compact、prompt-too-long recovery、tool-result budget、provider request 组装 |
| `agent-runtime` | 单 turn 循环：provider 调用、工具调度、**typed hook dispatch（`HookDispatcher` port）**、event mapping、pending event 输出 |
| `host-session` | session JSONL truth、投影/恢复/fork、input queue、child session、session plan、turn mutation、compaction、model selection hook |
| `plugin-host` | builtin/external plugin active snapshot、贡献校验、**executable hook binding（`BuiltinHookRegistry` + `dispatch_hooks`）**、capability dispatch、plugin backend 管理 |

### Adapters

| Crate | 职责 |
|---|---|
| `adapter-agents` | builtin/user/project agent profile 加载 |
| `adapter-llm` | OpenAI 兼容 Responses / Chat Completions provider |
| `adapter-mcp` | MCP stdio/HTTP/SSE 客户端，工具/提示/资源桥接 |
| `adapter-prompt` | prompt provider，按 prompt declaration 分层组装 |
| `adapter-skills` | Skill 发现、解析、builtin 打包、运行时物化 |
| `adapter-storage` | JSONL event store、config store、MCP settings store |
| `adapter-tools` | 文件、搜索、shell、skill、mode、todo、agent 协作等内置工具 |

### Entry / Composition

| Crate | 职责 |
|---|---|
| `server` | 唯一组合根、HTTP/SSE API、auth、runtime reload、governance surface、agent route bridge |
| `client` | 类型化 HTTP/SSE SDK，只依赖 `protocol` |
| `cli` | ratatui TUI，连接或拉起 server |
| `eval` | task YAML、隔离 workspace、提交 prompt、trace 提取、诊断、评分、report |
| `src-tauri` | Tauri 薄壳：sidecar 管理、窗口能力、系统对话框 |

## 依赖规则

强约束由 `scripts/check-crate-boundaries.mjs` 执行（R001–R013 规则）。

```text
entry -> client -> protocol -> core

server -> all crates

adapter-* -> contract/owner/base
owner/runtime -> contract/base
contract -> core
support -> core
core -> no workspace dependency
```

具体依赖（按 buildRules）：

| Crate | 允许的 workspace 依赖 |
|---|---|
| `core` | 无 |
| `protocol` | core, governance-contract |
| `prompt-contract` | core |
| `governance-contract` | core, prompt-contract |
| `tool-contract` | core, governance-contract |
| `llm-contract` | core, governance-contract, prompt-contract |
| `runtime-contract` | core, llm-contract, tool-contract |
| `support` | core |
| `context-window` | core, llm-contract, runtime-contract, tool-contract, support |
| `agent-runtime` | core, context-window, llm-contract, prompt-contract, runtime-contract, tool-contract |
| `plugin-host` | core, governance-contract, protocol, runtime-contract |
| `host-session` | core, support, agent-runtime, plugin-host, governance-contract, prompt-contract, runtime-contract, tool-contract |
| `adapter-*` | contract/owner/base（adapter 之间禁止横向依赖，`adapter-storage` 除外） |

必须通过以下命令验证：

```bash
node scripts/check-crate-boundaries.mjs --strict
```

## Server 内部结构

`server` 是唯一组合根，按职责拆分为以下子模块：

### `bootstrap/` — 组合根

- `runtime.rs`：`bootstrap_server_runtime()` 组装全部依赖，929 行
- `runtime_coordinator.rs`：`RuntimeCoordinator` 负责原子替换
- `builtin_plugins.rs`：内置 plugin descriptor 共享定义（权限、planning、composer）
- `governance.rs`：`ServerGovernanceService` 构建
- `capabilities.rs`：capability surface sync 和 tool search index
- `plugins.rs`：plugin 加载和 skill 发现
- `providers.rs`：LLM provider 和 config service 构建
- `mcp.rs`：MCP manager bootstrap
- `watch.rs`：profile watch runtime
- `deps.rs`：共享依赖

### `application/` — 应用用例层

- `agent/`：agent 协作编排（spawn / send / observe / close）
- `execution/`：root / subagent 执行配置
- `governance_surface/`：治理面装配（`GovernanceSurfaceAssembler` → `ResolvedGovernanceSurface` → `AppAgentPromptSubmission`）
- `lifecycle/`：治理模型（`AppGovernance`）、任务注册
- `root_execute.rs`：root execution 入口
- `error.rs` / `route_error.rs`：错误类型和 HTTP 映射

### `runtime_bridge/` — 桥接层

- `ports/`：应用层 port trait（`AppKernelPort`、`AgentKernelPort`、`AppSessionPort`、`AgentSessionPort`）
- `hook_dispatcher.rs`：`PluginHostHookDispatcher`（将 plugin-host 包装为 `HookDispatcher`）
- `capability_router.rs`：工具 → capability 路由
- `config_service.rs`、`governance_service.rs`、`mcp_service.rs`、`mode_catalog.rs`、`profile_service.rs`：服务桥接
- `agent_control_registry/`：多 parent delivery 注册中心
- `session_port/`：`AppSessionPort` 实现（adapter）
- `session_owner/`：session runtime bootstrap
- `runtime_owner.rs`：运行时可观测性和任务注册

### `http/` — 传输层

- `routes/`：Axum route handler（sessions / conversation / config / model / agents / mcp / composer / logs）
- `auth.rs`：bootstrap token 验证和 API 会话管理
- `mapper.rs`：DTO ↔ domain 映射
- `state.rs`：`AppState`（共享请求状态）

### `config/` — 配置管理

- config overlay、profile/model 选择、环境变量解析、MCP 配置

### `mode/` — 治理模式

- `catalog.rs`：mode 目录（builtin + plugin）
- `compiler.rs`：`GovernanceModeSpec` → `CompiledModeEnvelope` 编译
- `validator.rs`：mode 切换验证

### `read_model/` — 读模型投影

- `conversation/`：conversation block 投影（durable + live → `ConversationBlockFacts`）
- `terminal.rs`：terminal 输出投影

### `observability/` — 可观测性

- runtime metrics 收集和 governance snapshot

## 核心运行路径

### Prompt 提交

```text
frontend / cli
  -> astrcode-client / protocol DTO
  -> server http routes
  -> root_execute_service
  -> runtime_bridge/session_port adapter
  -> host-session accept / begin / persist / complete
  -> agent-runtime turn loop
  -> adapter-llm + adapter-tools
  -> host-session durable JSONL
  -> conversation projection + SSE
```

要点：

- `/api/sessions/{id}/prompts` 是普通会话 prompt 入口。
- `/api/v1/agents/{id}/execute` 是 root execution 入口。
- `root_execute_service` 负责把请求转为 root execution 请求，并应用治理输入。
- `host-session` 是 durable truth owner；server 只持有 bridge 和组合根资源。

### Agent 协作

```text
spawn / send / observe / close tool
  -> server application/agent orchestration
  -> runtime_bridge/agent_control_registry
  -> host-session child session / sub-run lineage
  -> agent-runtime child turn
  -> durable collaboration events
```

规则：

- child agent 表现为 child session。
- sub-run lineage 必须可持久化、可恢复、可查询。
- parent/child 通信必须通过协作端口和 input queue，不允许直接改内部状态。

### Config Reload

```text
POST /api/config/reload
  -> ServerGovernanceService
  -> config / MCP / plugin / skill reload
  -> candidate capability surface
  -> RuntimeCoordinator atomic replace
```

运行中存在 session 时拒绝 reload，避免执行中途语义漂移。candidate 构建失败时保留旧 active surface。

### Eval

```text
run-api-eval.mjs
  -> start astrcode-server
  -> exchange auth token
  -> cargo run -p astrcode-eval
  -> create session / submit prompt
  -> wait TurnDone in JSONL
  -> extract trace / diagnose / score / report
```

CI 只跑 smoke eval，验证框架不坏；真实 LLM 能力评估必须用 `npm run eval:api` 或直接调用 `astrcode-eval` 指向真实 server。

## Hook Dispatch

`runtime-contract` 定义 typed `HookEventPayload` 和 `HookEffect` 作为合同类型。
`plugin-host` 持有 `BuiltinHookRegistry`（内置 handler 注册中心）和 active snapshot 中的 `HookBinding`（executable hook 条目）。
`agent-runtime` 和 `host-session` 通过各自的 `HookDispatcher` / `HookDispatch` port 消费 hook 效果，不直接依赖 `plugin-host`。

### 事件类型（HookEventPayload）

| 事件 | 触发时机 |
|---|---|
| `Input` | 用户输入到达 |
| `Context` | 上下文窗口准备 |
| `BeforeAgentStart` | agent 开始执行前 |
| `BeforeProviderRequest` | LLM 请求发送前 |
| `ToolCall` | 工具调用前 |
| `ToolResult` | 工具结果返回后 |
| `SessionBeforeCompact` | 会话压缩前 |
| `ResourcesDiscover` | 资源发现 |
| `ModelSelect` | 模型选择 |
| `TurnStart` / `TurnEnd` | turn 生命周期 |

### 效果类型（HookEffect）

| 效果 | 语义 |
|---|---|
| `Continue` | 无操作，继续 |
| `TransformInput` / `HandledInput` / `SwitchMode` | 输入事件效果 |
| `ModifyProviderRequest` / `DenyProviderRequest` | LLM 请求效果 |
| `MutateToolArgs` / `BlockToolResult` / `RequireApproval` / `OverrideToolResult` | 工具效果 |
| `CancelTurn` | 取消当前 turn |
| `CancelCompact` / `OverrideCompactInput` / `ProvideCompactSummary` | 压缩效果 |
| `Diagnostic` / `ResourcePath` / `ModelHint` / `DenyModelSelect` | 辅助效果 |

### 数据流

```text
runtime event（tool_call / tool_result / before_provider_request 等）
  -> agent-runtime 构造 HookEventPayload typed variant
  -> HookDispatcher port（由 server hook_dispatcher 注入）
  -> plugin-host dispatch_hooks()
  -> 按事件过滤 HookBinding -> 按 priority 排序
  -> BuiltinHookRegistry 解析 executor -> 调用 handler
  -> handler 返回 Vec<HookEffect>
  -> dispatch_hooks 校验 effect 属于事件允许集合
  -> 按 dispatch mode + failure policy 处理
  -> HookDispatchOutcome 返回给 owner 应用
```

### 架构边界

- `agent-runtime` 只依赖 `runtime-contract` 和内部的 `HookDispatcher` trait。
- `host-session` 只依赖 `runtime-contract` 和内部的 `HookDispatch` trait。
- `plugin-host` 实现 handler 注册和 dispatch 引擎，但 agent-runtime/host-session 不直接引用 plugin-host。
- `server` 通过 `PluginHostHookDispatcher` 将 plugin-host dispatch core 包装为 `HookDispatcher` 注入。
- `HookContext` 只暴露 typed metadata、只读宿主视图，不暴露 `EventStore` 或 mutable session state。

### Builtin Plugin

builtin 能力（planning mode、权限、协作、composer、core tools）通过 builtin plugin contribution 注册：

- 每个 builtin plugin 声明 `PluginDescriptor` + 注册 executor
- descriptor 和 executor 一起 staging，绑定失败则保留旧 snapshot
- tool/hook/provider/command 统一通过 plugin-host snapshot 调度
- 函数式注册 helper（如 `registry.on_tool_call(...)`）降低 builtin plugin 作者心智成本

## Governance Mode

`GovernanceModeSpec` 是治理模式 DSL，位于 `governance-contract`。它描述：

- mode id / 展示名 / 说明
- capability selector
- tool policy / action policy
- child agent policy / execution policy
- prompt program / artifact contract / exit gate
- prompt hooks / transition policy

编译流程：`GovernanceModeSpec` → `CompiledModeEnvelope`（`mode/compiler.rs`）。

治理面装配：`*GovernanceInput` → `GovernanceSurfaceAssembler` → `ResolvedGovernanceSurface` → `AppAgentPromptSubmission`。

## Durable / Live 分层

### Durable

- 存储格式：JSONL append-only。
- 存储路径：`~/.astrcode/projects/<project>/sessions/<session-id>/session-<id>.jsonl`。
- 内容：session start、user input、assistant final、tool call/result、mode、compact、child/sub-run、turn done 等可回放事实。
- 用途：恢复、fork、baseline trace、eval scoring、历史回放。

### Live

- 传输：SSE。
- 内容：流式 delta、临时 thinking、control overlay、conversation block patch。
- 用途：低延迟 UI，不作为历史事实来源。

任何需要刷新后仍存在的状态，都必须能从 durable events 恢复。

### Conversation 投影

`read_model/conversation/` 负责 durable + live event → `ConversationBlockFacts` 的投影。支持 block 增量追加和 patch 语义，确保 SSE 断点续传正确。

## Prompt / Skill / MCP / Plugin

### Prompt

Prompt 由 `adapter-prompt` 根据 `prompt-contract` 组装。治理模式、agent profile、capability prompt、skill summary、MCP/plugin prompt 都必须通过声明式 prompt contribution 进入，不允许 adapter 私自拼 system prompt。

### Skill

加载来源优先级：

```text
builtin < mcp < plugin < user < project
```

Skill 目录格式：

```text
skill-name/
  SKILL.md
  references/
  scripts/
```

### MCP

MCP 作为 plugin-host 的外部贡献来源之一。工具名使用命名空间，避免与 builtin 冲突。MCP prompt/resource/tool 的 wire DTO 进入 server 后必须转换成内部 contract 类型。

### Plugin

plugin-host 维护 active snapshot：

1. 发现 manifest。
2. 校验全局唯一性。
3. 构建 candidate snapshot。
4. 启动 backend。
5. 原子提交。
6. 通过 capability router 调度。

## HTTP API 摘要

完整路由定义在 `crates/server/src/http/routes/mod.rs`。

| 分类 | 入口 |
|---|---|
| Auth | `POST /api/auth/exchange` |
| Session | `POST /api/sessions`, `POST /api/sessions/{id}/prompts`, `/compact`, `/fork`, `/interrupt`, `/mode` |
| Conversation | `GET /api/v1/conversation/sessions/{id}/snapshot`, `/stream`, `/slash-candidates` |
| Config / Model | `GET /api/config`, `POST /api/config/reload`, `POST /api/config/active-selection`, `/api/models...` |
| Agent | `GET /api/v1/agents`, `POST /api/v1/agents/{id}/execute`, `/api/v1/sessions/{id}/subruns/{sub_run_id}`, `/close` |
| MCP | `/api/mcp/status`, approval/reject/reconnect/server management |
| Logs | `POST /api/logs` |
| Bootstrap | `GET /__astrcode__/run-info`（浏览器开发桥接） |

## 验证矩阵

### 日常快速检查

```bash
cargo check --workspace
cargo test --workspace --exclude astrcode --lib
cd frontend && npm run typecheck
```

### 边界变更

```bash
node scripts/check-crate-boundaries.mjs --strict
cargo check --workspace
```

### 完整 CI 对齐

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
cd frontend && npm run typecheck && npm run lint && npm run format:check
```

### 真实模型评测

```bash
npm run eval:api -- --task-set eval-tasks/task-set.yaml --concurrency 1
```

## 当前允许的风险

| 风险 | 处理原则 |
|---|---|
| `context-window` 继续增长 | 到 compaction、tool-result budget、request shaping 出现明显独立演化时再拆 |
| plugin backend 形态较多 | 只保留 active snapshot 与调度边界稳定，不提前承诺所有 backend 产品化 |
| eval task 质量仍在建设 | CI 只验证 eval 框架；真实质量由 `eval:api` 报告驱动 |
| `runtime_bridge/session_port/adapter.rs` 仍较长 | 主要是 trait delegation boilerplate 和测试，结构合理但体量大，后续可按 port 拆分 |
