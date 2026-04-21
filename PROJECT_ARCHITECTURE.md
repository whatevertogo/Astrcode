# 项目架构总览

本文档是仓库级架构的权威说明。`README.md`、`docs/architecture/*` 与各专题文档可以展开局部细节，但不得与本文档的分层边界和依赖方向冲突。

## 架构核心原则：三层分离

session-runtime 内部存在两种根本不同的关注点，外加面向外部的一致接口。三层的规则各不相同，绝不可混合：

### 第一层：事件溯源层（发生了什么）

**规则**：纯函数、确定性、可回放、无副作用。

所有派生事实（phase、mode、turn terminal、active tasks、child session、input queue、conversation snapshot）必须能由事件流重新投影恢复。同一段投影逻辑只存在一个实现，不允许为增量、全量回放、checkpoint 恢复分别写三遍。

### 第二层：运行时状态层（正在发生什么）

**规则**：有副作用、有时序依赖、不可回放、不暴露给外部。

CancelToken 触发、running 标志（防止双 turn 并发）、LLM 流式响应累加、工具并发调度——这些是实时并发控制，不是从事件推断出来的投影。运行时状态只存在于 turn 执行期间，turn 结束后销毁，一切真相回归事件流。

### 第三层：外部接口层（外界看到什么）

**规则**：收纯数据、吐纯数据，永远不暴露运行时内脏。

所有外部扩展点（plugin、hook、capability、subscription、policy）通过纯数据交互：
- **订阅**：收到 `SessionEventRecord`，观察/记录，无副作用回流
- **Hook**：收到 `ToolHookContext`，返回 `ToolHookResultContext`（纯数据决策）
- **Capability**：通过 `CapabilitySpec` 声明，执行时收到 `ToolContext`，返回 `ToolExecutionResult`
- **Policy**：收到 `PolicyContext`，返回 `PolicyVerdict`
- **Plugin**：通过 `PluginManifest` 声明，通过 `CapabilitySpec` 注入能力

外部代码永远不应该看到 `CancelToken`、`AtomicBool`、`StdMutex<Option<ActiveTurnState>>` 等运行时类型。

### 三层的交互方向

```
运行时层（turn/）──写入事件──→ 事件溯源层（state/projections + query/）
                                      ↓
                               外部接口层（纯数据快照）
                                      ↓
                               application / server / plugin / hook
```

单向流动，不允许反向：投影层不能调运行时层，外部不能操作运行时状态。

## Crate 全览

项目包含 17 个 crate + 1 个 Tauri 桌面薄壳。按职责分为六层：

```
                        ┌─────────────┐
                        │ src-tauri   │  桌面薄壳
                        └──────┬──────┘
                               │
           ┌───────────────────┼───────────────────┐
           │                   │                   │
     ┌─────┴──────┐     ┌─────┴──────┐      ┌──────┴─────┐
     │   cli      │     │   server   │      │    eval    │
     │ (TUI 客户端)│     │ (组合根)    │      │ (离线评测) │
     └─────┬──────┘     └─────┬──────┘      └──────┬─────┘
           │                   │                    │
     ┌─────┴──────┐           │                    │
     │  client    │           │                    │
     │ (HTTP 传输) │           │                    │
     └─────┬──────┘           │                    │
           │                  │                    │
           │    ┌─────────────┼────────────┐       │
           │    │             │            │       │
           │  ┌─┴──────────┐ │  ┌─────────┴──┐  │
           │  │ application│ │  │   plugin    │  │
           │  │ (业务编排)  │ │  │ (插件运行时) │  │
           │  └─────┬──────┘ │  └──────┬──────┘  │
           │        │        │         │          │
           │  ┌─────┴──────┐ │  ┌──────┴───────┐ │
           │  │   kernel   │ │  │     sdk      │ │
           │  │ (能力聚合)  │ │  │ (插件 SDK)   │ │
           │  └─────┬──────┘ │  └──────┬───────┘ │
           │        │        │         │          │
           │  ┌─────┴──────────┴────────┴──────┐ │
           │  │        session-runtime          │ │
           │  │        (单会话执行引擎)           │ │
           │  └──────────────┬──────────────────┘ │
           │                 │                     │
           │    ┌────────────┼──────────────┐      │
           │    │            │              │      │
           │  ┌─┴──────┐ ┌──┴───────┐ ┌────┴────┐│
           │  │  core  │ │ protocol │ │adapter-* ││
           │  │(领域层) │ │(协议层)  │ │(7个适配器)││
           │  └────────┘ └──────────┘ └─────────┘│
           └─────────────────────────────────────┘
```

### 领域基础层

| Crate | 职责 | 依赖 |
|-------|------|------|
| **core** | 领域协议和跨 crate 共享的纯数据模型。定义所有 port trait（`EventStore`、`LlmProvider`、`Tool`、`PromptProvider` 等）、领域事件（`StorageEventPayload`、`AgentEvent`）、能力模型（`CapabilitySpec`）、配置模型、治理模式 DSL。是整个项目的类型基石。 | 无项目内依赖 |
| **protocol** | 纯数据契约层。定义 HTTP DTO 和插件 JSON-RPC 消息格式，是 server↔client、server↔plugin 之间的序列化协议。不包含业务逻辑。 | core |

### 运行时层

| Crate | 职责 | 依赖 |
|-------|------|------|
| **kernel** | 运行时能力聚合层。组合 LlmProvider + PromptProvider + ResourceProvider + CapabilityRouter + AgentControl 为统一 `Kernel`。`KernelGateway` 收敛四个 provider 为单一门面；`AgentControl` 管理多 agent 生命周期编排、父子树、收件箱、父投递队列；`KernelAgentSurface` 提供面向编排层的稳定视图。 | core |
| **session-runtime** | 单会话执行引擎和事实边界。管理 turn 生命周期、事件投影、compact/恢复、流式对话。内部分为三层：运行时执行层（`turn/`）、事件溯源层（`state/projections`）、读投影层（`query/`）。详见下方"session-runtime 内部架构"章节。 | core, kernel |
| **plugin** | 宿主侧插件运行时。管理插件子进程生命周期（supervisor）、JSON-RPC over stdio 通信、能力路由桥接、流式执行。是外部插件接入 Astrcode 的基础设施。 | core, protocol |
| **sdk** | 插件开发 SDK。为插件开发者提供 Rust API：`ToolHandler` 注册工具、`HookRegistry` 注册钩子、`PluginContext` 访问调用上下文、`StreamWriter` 发送流式响应。插件通过 SDK 与宿主交互，不直接依赖 core 或 runtime。 | core, protocol |

### 编排层

| Crate | 职责 | 依赖 |
|-------|------|------|
| **application** | 业务编排层，唯一的用例入口。通过 port trait 与 session-runtime 和 kernel 解耦。编排根代理执行、子代理 spawn/send/observe/close 四工具、child turn 终态收口、parent delivery 唤醒调度、governance surface 计算、workflow/plan 状态机。 | core, kernel, session-runtime |

### 适配器层

| Crate | 职责 | 依赖 |
|-------|------|------|
| **adapter-agents** | Agent Profile 加载：从 builtin/用户级/项目级目录读取 Markdown YAML frontmatter + 纯 YAML，产出 `AgentProfileRegistry` | core |
| **adapter-llm** | 多 LLM 后端统一抽象（Anthropic Claude + OpenAI 兼容 API）：流式 SSE 响应累加、错误分类、指数退避重试 | core |
| **adapter-mcp** | MCP 服务器连接管理：工具/prompt/资源桥接，将外部 MCP 服务器能力注册到 Astrcode 能力路由 | core, adapter-prompt |
| **adapter-prompt** | Prompt 组装管线：贡献者模式，每个 `PromptContributor` 生成一段 Block，`PromptComposer` 收集/去重/拓扑排序/渲染，产出最终 `PromptPlan` | core |
| **adapter-skills** | Skill 资源发现：Markdown 解析、builtin/用户/项目分层 catalog 合并 | core |
| **adapter-storage** | 本地文件系统 JSONL 事件日志存储、文件锁互斥写入、会话仓库、配置持久化 | core |
| **adapter-tools** | 内置工具集（readFile、writeFile、editFile、grep、shell 等）+ Agent 协作工具（spawn、send、observe、close），实现 `Tool` trait | core |

### 接入层

| Crate | 职责 | 依赖 |
|-------|------|------|
| **server** | 唯一组合根。基于 axum 的 HTTP 服务端，组装 application、session-runtime、kernel 与所有 adapter。负责 bootstrap 装配和 HTTP 协议映射，不承载业务真相。 | 全部 |
| **cli** | TUI 客户端。基于 ratatui 的终端交互界面，通过 `client` crate 与服务端通信。 | client, core |
| **client** | HTTP 传输客户端。基于 reqwest 封装认证交换、会话管理、对话流式传输。 | protocol |
| **eval** | 离线评测框架。包含任务定义、trace 模型、runner、diagnosis 模块，支持 agent 行为的自动化测试与诊断。 | core, protocol |

### 桌面薄壳

| Crate | 职责 | 依赖 |
|-------|------|------|
| **src-tauri** | Tauri 桌面端薄壳。通过 `astrcode-server` 启动后端服务，前端 UI 通过 HTTP 与后端交互。不承载业务逻辑。 | server |

## Crate 分层（详细边界）

### `core` — 领域协议和纯数据模型

- 定义跨 crate 共享的类型、trait、port。
- `CapabilitySpec` 是运行时内部能力语义真相。
- `WorkflowDef`、`WorkflowPhaseDef` 等协议也属于这一层。
- **不包含运行时逻辑**：回放算法、文件 I/O、进程检测不属于 core。Core 定义类型，不实现算法。
- **不依赖** `application`、`session-runtime` 或任何 adapter。

core 中需要警惕的边界：
- `TurnProjectionSnapshot` 仅被 session-runtime 消费，属于 session-runtime 内部概念。
- `InputQueueProjection::replay_index()` 包含回放算法，应归入 session-runtime。
- `tool_result_persist` 执行文件 I/O，应归入 adapter。
- `RuntimeCoordinator` 包含有状态实现，应归入 application。
- `agent/mod.rs`（~60 个公开类型）需要按关注点拆分（types、collaboration、delivery、lineage）。

### `kernel` — 运行时能力聚合层

- 组合根：通过 `KernelBuilder` 将 LlmProvider + PromptProvider + ResourceProvider + CapabilityRouter + AgentControl 组装为 `Kernel`。
- 门面：`KernelGateway` 收敛四个 provider 为统一入口，session-runtime 不直接持有各 provider。
- 控制平面：`AgentControl` 提供多 agent 的生命周期编排、父子树管理、收件箱通信、父投递队列。
- Anti-corruption layer：`KernelAgentSurface` 将 `AgentControl` 内部 API 整形为编排层友好的稳定接口。
- 只依赖 `core`。不重新定义 core 的任何 trait。

### `session-runtime` — 单会话执行引擎

是单 session 执行与恢复的 authoritative truth。内部模块按三层原则划分：

#### `state/` — 事件溯源基础设施

**应该只做**：事件追加、投影计算、最近事件缓存、checkpoint 恢复。

- `SessionState` 持有 `ProjectionRegistry` + `SessionWriter` + `broadcaster`。
- `ProjectionRegistry` 按投影域组织：phase、agent、mode、children、tasks、input_queue、turns、cache。每个域应是独立 struct，`apply()` 委托分发而非一个大 if-else。
- `SessionWriter` 封装存储后端写入抽象。
- `RecentSessionEvents` / `RecentStoredEvents` 提供滑动窗口缓存。

**不应该做**：
- 不持有 `TurnRuntimeState`（运行时状态机应属于 `turn/` 模块）。
- 不包含命令处理器（`InputQueueEventAppend`、`append_input_queue_event` 应属于 `command/`）。
- 不提供绕过事件溯源的命令式写入（如 `upsert_child_session_node`）。

#### `turn/` — 运行时执行层

**应该只做**：turn 生命周期管理、LLM 调用、工具执行、流式处理。

- `TurnRuntimeState`（prepare/complete/interrupt/cancel）属于此模块，不属于 `state/`。
- `runner/` 负责单步循环编排（prompt → LLM → 工具/停止）。
- `submit.rs` 只做提交入口和协调，终结持久化和 SubRun 事件构造应拆为独立模块。
- 所有压缩后事件组装（proactive/reactive/manual）应抽取为共享函数，消除三处重复。

**不应该做**：
- 不包含只读查询（`replay.rs` 应属于 `query/`）。
- 不反向调用 `query/` 的方法（`current_turn_messages` 应为 `SessionState` 的投影方法）。

#### `query/` — 纯读投影层

**应该只做**：从事件流或投影缓存计算只读快照。

- `service.rs` 是纯协调器：拿到 state → 调投影函数 → 返回结果。
- `turn.rs` 是 turn 终态投影的唯一权威位置（合并当前分散在 `state/`、`query/`、`service.rs` 中的逻辑）。
- `conversation.rs` 承载会话流式投影。
- `agent.rs`、`terminal.rs`、`transcript.rs` 各自职责单一。

**不应该做**：
- 不包含异步事件监听循环（`wait_for_turn_terminal_snapshot` 的等待逻辑应在 `turn/` 内部或独立 watcher）。
- 不做数据分页或输入标准化（应提取为共享辅助）。

#### `command/` — 写入口

**应该只做**：接收写操作请求，委托 `state/` 完成事件追加。

- `compact_session()` 的立即执行路径应下沉到 `turn/`，command/ 只负责"提交 compact 请求"。

#### `context_window/` — 上下文窗口管理

- 提供 compact、prune、micro_compact、file_access、token_usage 等能力。
- 明确不承担最终请求组装（由 `turn/request.rs` 编排）。
- 对 `turn/` 单向依赖，`turn/` 通过 `request.rs` 汇聚所有 context_window 子模块。

#### `actor/` — SessionActor

- `SessionState` 的轻量容器 + 恢复入口。不包含写入逻辑。

#### `observe/` — 纯数据类型

- 只定义 session observe 的数据 shape（filter、scope、source）。
- 投影算法在 `query/`，类型定义在 `observe/`。

### `application` — 业务编排层

- 是唯一的业务编排入口。
- 解释 active workflow、phase signal、phase overlay、artifact bridge 与 mode 切换顺序。
- 通过 port trait（`AppSessionPort`、`AgentSessionPort`、`AppKernelPort`、`AgentKernelPort`）与 session-runtime 和 kernel 解耦。

**边界纪律**：
- port trait 方法签名中不应暴露 session-runtime 内部类型（`TurnTerminalSnapshot`、`ProjectedTurnOutcome` 等）。需要跨层传递的信息应在 core 中定义稳定类型，或在 port impl 中做映射。
- `lib.rs` 不应批量 re-export session-runtime 的类型穿透到上层。
- `CapabilityRouter`（kernel 具体 struct）不应出现在 application 公共 API 中。
- 不直接操作 session-runtime 的 `append_and_broadcast`、`prepare_execution` 等内部方法。

### `server` — 组合根与 HTTP 路由

- 是唯一组合根，组装 `application`、`session-runtime`、`kernel` 与各 adapter。
- 不承载业务真相，只负责装配和协议映射。

**边界纪律**：
- HTTP 路由不应直接 import session-runtime 的 `Conversation*Facts`、`ConversationStreamProjector`、`ForkPoint` 等内部类型。所有业务交互通过 `application` 的用例方法。
- 不直接调用 `normalize_working_dir` 等 session-runtime 工具函数。
- 测试不应直接操作 `SessionState::append_and_broadcast`。

## mode envelope 与 workflow phase 的关系

- `mode` 负责治理约束，回答"这一轮允许做什么、如何做"。
- `workflow phase` 负责业务语义，回答"当前处于正式流程的哪一段、下一步如何迁移"。
- 同一个 `mode_id` 可以被多个 phase 复用。
- workflow 迁移必须通过显式 `transition` 与 `bridge` 建模，不能散落在提交入口的 plan-specific if/else 里。

## 依赖方向

仓库级依赖方向保持如下不变式：

- `server` 是组合根，只通过 `application` 层消费业务逻辑，仅在 bootstrap 中直接引用 `kernel` 和 adapter。
- `application` 只依赖 `core`、`kernel`、`session-runtime`。
- `session-runtime` 只依赖 `core`、`kernel`。
- `kernel` 只依赖 `core`。
- `protocol` 只依赖 `core`。
- `adapter-*` 只依赖 `core`（互不依赖）。
- `src-tauri` 是桌面薄壳，不承载业务逻辑。

## 事件与恢复语义

- event log 是执行时间线的 durable truth，append only，不改不删。
- 所有派生事实必须能由事件投影恢复。
- display `Phase` 只由 durable event 投影驱动，不允许被运行时代码直接写入。
- workflow instance state 是独立于 runtime checkpoint 的显式持久化状态；workflow 恢复失败时允许降级到 mode-only 路径。
- 投影逻辑遵循唯一实现原则：同一段投影（如 turn 终态、compact 后事件组装）只存在一个实现，增量/全量/恢复三种路径共享同一份投影函数。

## 文档关系

- 本文档：仓库级分层边界与依赖方向的权威约束。
- `README.md`：项目介绍和对外说明。
- `docs/architecture/crates-dependency-graph.md`：crate 依赖图和结构快照。
- `CLAUDE.md`：开发者工作流、常用命令、代码规范。
