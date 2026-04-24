# Astrcode 项目架构

本文档描述仓库的**当前实际架构**。目标不是解释历史，而是约束未来实现。

## 架构原则

- 不维护向后兼容。发现旧边界阻碍正确架构时，优先一次性迁移调用点，而不是保留兼容 re-export。
- `server` 是唯一组合根。它可以依赖所有实现 crate，用于装配、热替换和 HTTP 暴露，但不能成为长期业务真相的持有者。
- `core` 只承载稳定、无副作用、无宿主策略的共享语义对象和事件数据模型。禁止放入工具 trait、LLM/runtime 边界、治理默认实现、prompt 契约、有状态投影翻译或环境读取。
- `*-contract` crate 只承载窄领域契约。契约 crate 不能包含宿主耦合默认实现、宿主 I/O、adapter 逻辑或运行时编排；少量纯策略兜底实现必须无状态、无副作用，避免变成第二个 `core`。
- owner crate 持有各自真相：`host-session` 持有 durable session truth，`plugin-host` 持有插件 truth，`agent-runtime` 只负责编排单 turn 执行，`context-window` 负责上下文窗口与请求整形。
- adapter 之间禁止横向依赖。跨 adapter 共享的类型必须下沉到对应 contract crate。
- durable/live 分层必须清晰：JSONL durable events 是刷新、恢复、fork、历史回放的权威事实；SSE live events 只承担低延迟体验和临时草稿。
- 类型边界显式转换。不同层的 `ModeId`、prompt、tool、runtime 事件不得通过根 crate re-export 偷渡。

## Crate 一览

仓库当前包含 `src-tauri` 薄壳在内的多 crate 工作区，核心分层如下：

```text
┌─────────────────────────────────────────────────────────────┐
│ shell / entry                                                │
│ src-tauri (Tauri 桌面壳) │ cli (TUI) │ eval (评测框架)       │
├─────────────────────────────────────────────────────────────┤
│ client (HTTP SDK)                                            │
├─────────────────────────────────────────────────────────────┤
│ server (唯一组合根 + HTTP 路由)                              │
├─────────────────────────────────────────────────────────────┤
│ adapters                                                     │
│ adapter-agents │ adapter-llm │ adapter-mcp │ adapter-prompt  │
│ adapter-skills │ adapter-storage │ adapter-tools             │
├─────────────────────────────────────────────────────────────┤
│ owners / runtime                                             │
│ host-session │ plugin-host │ agent-runtime │ context-window  │
├─────────────────────────────────────────────────────────────┤
│ contracts                                                    │
│ prompt-contract │ governance-contract │ tool-contract        │
│ llm-contract │ runtime-contract                              │
├─────────────────────────────────────────────────────────────┤
│ base                                                        │
│ core │ protocol │ support                                    │
└─────────────────────────────────────────────────────────────┘
```

---

## 各 Crate 职责

### `astrcode-core` — 共享语义层

跨 crate 共享的稳定值对象和事件模型：

- **强类型 ID**：`SessionId` / `TurnId` / `AgentId` / `SubRunId`
- **稳定语义对象**：`CapabilitySpec`、动作/能力描述、消息与阶段枚举
- **事件数据模型**：`StorageEvent`、`AgentEvent` 以及可回放的 durable payload
- **基础对象**：`CancelToken`、`AstrError`、环境变量名常量

`core` 不依赖任何其他工作区 crate，也不主动读取宿主环境。环境解析、路径探测和文件系统 I/O 放在 `support` 或组合根。

### Contract Crates

契约 crate 只定义跨层边界，不拥有实现：

- `astrcode-prompt-contract`：`PromptDeclaration`、prompt source/kind/render target、prompt cache hints/diagnostics、prompt layer/fingerprint。
- `astrcode-governance-contract`：`ModeId`、mode DSL、tool policy、`PolicyEngine` trait、`AllowAllPolicyEngine`、`ModelRequest`、`SystemPromptBlock`。
- `astrcode-tool-contract`：`Tool`、`ToolContext`、`ToolEventSink`、工具元数据、工具结果、工具输出 delta sender。
- `astrcode-llm-contract`：`LlmProvider`、`LlmRequest`、`LlmOutput`、`LlmEvent`、`LlmEventSink`、`ModelLimits`、`LlmUsage`。
- `astrcode-runtime-contract`：`RuntimeHandle`、runtime boundary traits、`RuntimeTurnEvent`。

### `astrcode-context-window` — 上下文窗口

从 `agent-runtime` 拆出的请求整形子系统：

- LLM 驱动 compaction、prompt-too-long recovery、contract retry 与 sanitization。
- tool-result budget、超大工具输出的文件引用恢复。
- `assemble_runtime_request`，将 session state、prompt、tool result 与模型限制组装成 provider 请求。

仅允许依赖 `core`、`llm-contract`、`runtime-contract`、`tool-contract`、`support`。

### `astrcode-protocol` — 传输层 DTO

纯数据契约层，所有类型为可序列化 DTO，不含业务逻辑：

- `capability/`：`CapabilityWireDescriptor`、`InvocationContext`、`PeerDescriptor`。
- `http/`：HTTP API 请求/响应 DTO，包含 auth、session、conversation、agent、config、model、SSE 事件信封。
- `plugin/`：JSON-RPC 插件协议。

仅允许依赖 `core` 与对外传输需要暴露的 contract crate（当前为 `governance-contract`）。

### `astrcode-support` — 宿主环境工具

不应落在 `core` 中的环境依赖工具：

- `hostpaths`：ASTRCODE_HOME / 项目目录解析。
- `shell`：跨平台 shell 检测。
- `tool_results`：大型 tool 输出的磁盘持久化。

仅允许依赖 `core`。

### `astrcode-agent-runtime` — 最小执行内核

单 turn / 单 agent 的 live 执行编排：

- **Turn 循环**：初始化 → hook dispatch → provider 调用 → tool dispatch → 输出 → 终结。
- **Provider 调用**：消费 `llm-contract`，不定义 LLM 公共契约。
- **Tool 调度**：消费 `tool-contract`，支持并行执行与实时流式输出。
- **Hook 调度**：支持 Continue / Block / CancelTurn / AugmentPrompt / Diagnostic 效果。
- **Pending event 编排**：将运行时事件交给 host/session bridge 处理 durable 与 live 投影。

仅允许依赖 `core`、`context-window`、`llm-contract`、`prompt-contract`、`runtime-contract`、`tool-contract`。

### `astrcode-host-session` — Session Owner

统一承接 durable truth 和 host use-case：

- **事件持久化**：`SessionWriter` 双路径写入，生产使用 `EventStore` 异步追加，测试可同步写入。
- **恢复与回放**：checkpoint + tail events 追放。
- **投影 / 查询 / 观察**：`ProjectionRegistry` 维护 phase、agent state、mode、child node、active task、input queue 等投影。
- **Turn 变更**：accept → begin → persist inputs → persist runtime events → complete/interrupt。
- **EventTranslator**：作为 session durable/live 投影实现细节，不能回流到 `core`。
- **多 Agent 协作**：child session、sub-run lineage、输入队列和协作事件统一持久化。
- **Session Plan**：结构化计划生命周期。

仅允许依赖 `core`、`support`、`agent-runtime`、`plugin-host`、`governance-contract`、`prompt-contract`、`runtime-contract`、`tool-contract`。

### `astrcode-plugin-host` — 统一插件宿主

builtin / external plugin 的统一管理：

- **描述符模型**：tools、hooks、providers、resources、commands、prompts、skills、themes、modes。
- **校验与快照**：全局唯一性约束、候选快照、原子 commit / rollback。
- **后端统一**：Builtin / Process / Command / Http 统一到 `PluginRuntimeHandleRef`。
- **能力调度管线**：binding → plan → readiness check → dispatch。
- **Hook Bus**：优先级排序、dispatch mode、failure policy。
- **传输层**：JSON-RPC over stdio。

仅允许依赖 `core`、`protocol`、`governance-contract`、`support`。

### Adapter Crates

7 个 adapter 遵循端口-适配器模式，实现各自的上层 trait：

| Crate | 实现的 Port | 职责 |
|---|---|---|
| `adapter-agents` | 无（纯数据注册表） | Agent profile 多源加载（builtin < user < project），YAML/Markdown 解析 |
| `adapter-llm` | `LlmProvider`（llm-contract） | OpenAI 兼容 API，Chat Completions + Responses API，SSE 流式，指数退避重试，prompt cache 诊断 |
| `adapter-mcp` | `ResourceProvider`（plugin-host）/ `CapabilityInvoker`（core） | MCP JSON-RPC 客户端，工具/提示/资源桥接，直接产出 `prompt-contract::PromptDeclaration`，工具名命名空间 `mcp__{server}__{tool}` |
| `adapter-prompt` | `PromptProvider`（host-session） | 四层缓存架构、贡献者模式、波拓扑排序，直接消费 `prompt-contract` |
| `adapter-skills` | `SkillCatalog`（core） | 多源技能叠加，编译时 builtin 打包，运行时资产物化 |
| `adapter-storage` | `EventStore` + `SessionManager`（host-session）/ `ConfigStore` + `McpSettingsStore`（core） | JSONL 追加日志、原子文件写入、OS 级文件锁、checkpoint 恢复 |
| `adapter-tools` | `Tool`（tool-contract）× 15+ | 文件操作、shell、搜索、Skill 加载、任务管理、模式切换、Agent 协作 |

### `astrcode-server` — 组合根 + HTTP 服务

唯一允许同时依赖所有 adapter、owner、runtime 与 contract crate 的地方：

- **Bootstrap**：配置 → MCP → 插件 → 工具索引 → 能力快照 → session 运行时 → agent 运行时包 → governance → `ServerRuntime`。
- **Runtime Coordinator**：原子热替换 runtime surface。
- **Ports 模块**：六边形架构端口接口和 bridge 适配器。
- **Governance Surface**：每 turn 治理决策、审批策略、子 agent 委托、协作引导。
- **Capability Router**：本地 builtin + 动态外部双层能力模型。
- **HTTP 路由**：Auth / Session CRUD / Conversation SSE / Config / Model / Agent / MCP / Logs。
- **Lifecycle**：追踪 turn 和 subagent 任务句柄，关闭时批量终止。

### `astrcode-client` — HTTP SDK

类型化异步 Rust SDK：

- `AstrcodeClient<T>` 泛型 transport，默认 Reqwest transport。
- 覆盖 session CRUD、prompt 提交、conversation SSE、model 查询、compact、mode 切换。
- SSE 解析与 `ConversationStream`。

仅依赖 `protocol`。

### `astrcode-cli` — 终端 UI

基于 ratatui / crossterm 的 TUI：

- 事件循环、流式对话、slash 命令面板、model 选择、session 切换、mode 切换、thinking 动画、markdown 渲染。
- 自动发现或 spawn 服务器。

依赖 `client`、`core`、`support`。

### `astrcode-eval` — 评测框架

离线 agent 质量评测：

- YAML 任务定义与多维评分。
- 隔离工作区 → 创建 session → 提交 prompt → 轮询完成 → 提取 trace → 诊断 → 评分。
- 失败模式检测。

依赖 `core`、`protocol`、`support`。

### `src-tauri` — Tauri 桌面壳

Tauri v2 薄壳，不含业务逻辑：

- 服务器生命周期管理。
- 桌面前端模式检测。
- 系统对话框。

仅依赖 `core`。

---

## 依赖方向

### 分层方向

```text
entry crates
  └─ client
      └─ protocol
          └─ core

server
  ├─ adapters
  ├─ owner/runtime crates
  ├─ contract crates
  └─ base crates

adapters
  ├─ contract crates
  ├─ owner ports
  └─ core/support

owner/runtime crates
  ├─ contract crates
  └─ core/support

contract crates
  └─ core

support
  └─ core

core
  └─ 无工作区依赖
```

adapter 到 adapter 的依赖一律禁止。需要共享 prompt、tool、LLM、runtime 或 governance 类型时，必须放入对应 contract crate。
部分 contract crate 可以依赖更底层 contract，具体以“强约束”表为准。

### 强约束

| Crate | 允许依赖 |
|---|---|
| `core` | 无（零工作区依赖） |
| `protocol` | `core`、`governance-contract` |
| `support` | `core` |
| `agent-runtime` | `core`、`context-window`、`llm-contract`、`prompt-contract`、`runtime-contract`、`tool-contract` |
| `plugin-host` | `core`、`protocol`、`governance-contract`、`support` |
| `host-session` | `core`、`support`、`agent-runtime`、`plugin-host`、`governance-contract`、`prompt-contract`、`runtime-contract`、`tool-contract` |
| `prompt-contract` | `core` |
| `governance-contract` | `core`、`prompt-contract` |
| `tool-contract` | `core`、`governance-contract` |
| `llm-contract` | `core`、`governance-contract`、`prompt-contract` |
| `runtime-contract` | `core`、`llm-contract`、`tool-contract` |
| `context-window` | `core`、`llm-contract`、`runtime-contract`、`tool-contract`、`support` |
| `adapter-agents` | `core`、`support` |
| `adapter-llm` | `core`、`llm-contract`、`prompt-contract` |
| `adapter-mcp` | `core`、`prompt-contract`、`plugin-host`、`support` |
| `adapter-prompt` | `core`、`governance-contract`、`host-session`、`prompt-contract`、`support` |
| `adapter-skills` | `core`、`support` |
| `adapter-storage` | `core`、`host-session`、`support` |
| `adapter-tools` | `core`、`governance-contract`、`host-session`、`tool-contract`、`support` |
| `server` | 所有 crate |
| `client` | `protocol` |
| `cli` | `client`、`core`、`support` |
| `eval` | `core`、`protocol`、`support` |
| `src-tauri` | `core` |

### 边界备注

- `protocol -> governance-contract` 是传输 DTO 暴露 mode 信息的显式例外，不能扩展成任意 contract 依赖。
- `host-session -> agent-runtime` 只用于 runtime event 与 turn orchestration 的宿主集成，不能把 agent-runtime 的执行细节扩散回 session owner。
- `context-window` 已经是 `agent-runtime` 的外置子系统，不得把 compaction、tool-result 文件恢复或 request shaping 移回 turn loop。

---

## 核心设计模式

### 事件溯源（Event Sourcing）

Session 使用 JSONL 追加日志持久化事件流：

- **写入**：`SessionWriter` → `EventStore::append()` → JSONL 文件。
- **广播**：durable event 先更新投影，再翻译为 live event 广播到订阅者。
- **恢复**：checkpoint + tail events 追放。
- **Compaction**：LLM 驱动摘要替换旧消息前缀，并自动 checkpoint。

durable JSONL 是权威事实。LLM token/thinking delta 属于 live 草稿，默认不逐 token 写入 JSONL；最终 assistant 文本、最终 reasoning、工具事件、错误和完成事件必须持久化。

### 端口-适配器（Hexagonal Architecture）

系统边界通过 trait 端口定义，adapter 提供具体实现：

| 端口（定义于） | 适配器 |
|---|---|
| `Tool`（tool-contract） | `adapter-tools` 中 15+ 工具 |
| `LlmProvider`（llm-contract） | `adapter-llm` OpenAI 兼容 |
| `EventStore`（host-session） | `adapter-storage` JSONL |
| `PromptProvider`（host-session） | `adapter-prompt` 四层缓存构建器 |
| `SkillCatalog`（core） | `adapter-skills` 多源叠加 |
| `SubAgentExecutor`（host-session） | `server` 中注入 |
| `CollaborationExecutor`（host-session） | `server` 中注入 |
| `ResourceProvider`（plugin-host） | `adapter-mcp` MCP 资源桥接 |
| `ConfigStore`（core） | `adapter-storage` 文件系统 |
| `BuiltinCapabilityExecutor`（plugin-host） | `server` 中注册 |

### 插件系统

统一四后端模型（Builtin / Process / Command / Http）：

1. **加载**：扫描 plugin manifest。
2. **校验**：全局唯一性约束。
3. **暂存**：构建候选快照。
4. **启动后端**：builtin 使用 in-process handle，external 使用子进程或 HTTP 后端。
5. **提交**：原子替换 active snapshot。
6. **调度**：binding → plan → readiness check → dispatch。

热替换通过 `RuntimeCoordinator` 原子执行。

### 多 Agent 协作

- 一个 session 就是一个 agent，child agent 表现为 child session。
- `SubRunHandle` 承载完整 lineage。
- 协作通过 `SubAgentExecutor` / `CollaborationExecutor` trait 注入。
- Durable truth 统一归 `host-session`。
- 输入队列状态机、协作事件和取消传播都必须可回放。

### Prompt 组装

四层缓存友好架构：

| 层 | 稳定性 | 内容 |
|---|---|---|
| Stable | 极少变化 | Identity + Environment + ResponseStyle |
| SemiStable | 配置变更时失效 | AgentProfile + CapabilityPrompt + SkillSummary |
| Inherited | 按 prompt declaration 变化 | Plugin / MCP / 用户 prompt 声明 |
| Dynamic | 每 turn 变化 | Workflow 示例 |

贡献者模式 + 波拓扑排序解决依赖，最终渲染为 `SystemPromptBlock` 送入 LLM。

### Hook 总线

Hook bus 是唯一扩展总线。Governance、tool policy、model selection 通过 hook bus dispatch，避免 adapter 或 runtime 私自绕过治理面。

---

## 治理模式 DSL

`GovernanceModeSpec`（governance-contract）定义声明式模式 DSL：

- `mode_id` / `display_name` / `description`
- `capabilities`：`CapabilitySelector`
- `tool_policy`：按工具与能力描述限制
- `action_policies`：读、写、shell、network、agent spawn 等动作策略
- `child_agent_policy` / `execution_policy`
- `prompt_program` / `artifact_contract` / `exit_gate`
- `prompt_hooks` / `transition_policy`

运行时通过治理 surface 解析为每 turn 上下文快照。mode 的 durable 表达和 runtime 表达允许不同类型，但转换必须发生在明确边界：

- session durable events 与历史回放使用 `core` 中的 mode 事件数据。
- runtime、tool policy、prompt 组装使用 `governance-contract` 中的 mode 契约。
- 转换点应位于 `server`、`host-session` 或工具事件桥接处，禁止用旧 re-export 隐式兼容。

---

## 验证要求

每次涉及边界变更时，至少验证：

```bash
node scripts/check-crate-boundaries.mjs --strict
cargo check --workspace
cargo test --workspace --exclude astrcode --lib
```

完整 CI 检查见仓库 `AGENTS.md` 的“常用命令”。

---

## 当前风险与例外

| 项目 | 约束 |
|---|---|
| `protocol -> governance-contract` | 仅用于传输层 mode DTO，不得扩展为协议层依赖任意运行时契约 |
| `host-session -> agent-runtime` | 只允许用于 turn/runtime 事件宿主集成，不得让 session owner 直接持有 provider/tool 细节 |
| `context-window` 体量 | 允许先作为整体 crate 存在；继续增长时应按 compaction、tool-result budget、file recovery 再拆分 |
| 插件 HTTP 后端 | 已有后端形态，只有实际产品需求出现时再补齐实现 |
