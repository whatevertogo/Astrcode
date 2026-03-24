# AstrCode 项目结构报告

> 生成时间：2026-03-24
> 架构指南版本：Draft

---

## 一、架构核心理念

### 项目定位

AstrCode 是一个**面向编码场景的本地智能体平台**。

其核心不是某个固定 Agent，而是一个可扩展的 **协议内核 + SDK + 插件运行时**。

**目标边界**：
- 前端可替换（Desktop / Web / TUI）
- 运行时可替换（Native / LangChain / 其他编排）
- 插件可独立开发、独立部署、独立演进
- 会话、事件、工具、状态始终由平台统一管理

### 核心原则

| 原则 | 含义 |
|------|------|
| **平台为主** | 平台是系统真源，统一管理会话、事件流、工具调用、权限与策略、插件注册与生命周期 |
| **协议优先** | 跨层通信先定义协议，再定义实现。所有跨进程、跨语言、跨前端交互都基于统一协议 |
| **插件优先** | Agent、Tool、Context Provider、Memory Provider 都应尽量插件化。内核只保留最小必要能力,并且插件在保证安全的情况下能使用core的所有能力|
| **Server is the Truth** | 前端不是状态真源。会话、消息、任务、工具调用记录都以服务端/内核侧状态为准 |
| **UI 可替换** | Tauri/Desktop、Browser、CLI/TUI 都应复用同一套后端协议与核心能力 |

### 一句话总结

> AstrCode 的核心不是"一个很强的 Agent"，而是一个以 Server 为真源、以协议为边界、以插件为扩展单位、以 Runtime 为可替换执行层的编码智能体平台。

---

## 二、总体架构

```
┌──────────────────────────────────────┐
│              Frontends               │
│   Desktop (Tauri) / Web / CLI/TUI   │
└──────────────────┬───────────────────┘
                   │ HTTP / SSE / WS
┌──────────────────▼───────────────────┐
│            AstrCode Server           │
│ session / event / tool / state truth │
└──────────────────┬───────────────────┘
                   │ internal contracts
┌──────────────────▼───────────────────┐
│             Core Kernel              │
│ registry / router / policy / store   │
└───────────────┬───────────┬──────────┘
                │           │
        ┌───────▼──────┐  ┌─▼────────────────┐
        │ Agent Runtime │  │ Plugin Runtime   │
        │ native/planner│  │ stdio / websocket│
        └───────┬──────┘  └────────┬──────────┘
                │                  │
        ┌───────▼──────┐   ┌───────▼────────┐
        │ Built-in     │   │ External       │
        │ capabilities │   │ plugins / SDK  │
        └──────────────┘   └────────────────┘
```

### 分层职责

| 层级 | 职责 | 不负责 |
|------|------|--------|
| **Frontends** | 输入输出、交互呈现、事件订阅、局部缓存与乐观展示 | 会话真状态、工具调度、插件管理、核心业务决策 |
| **Server** | 提供统一 API、持有会话真状态、输出 SSE/流式事件、协调前端与内核 | — |
| **Core Kernel** | Session 管理、EventStore/Projection、ToolRegistry、CapabilityRouter、Policy/Permission、PluginRegistry、Runtime 编排 | 具体智能体策略实现 |
| **Agent Runtime** | 单轮生成、工具调用循环、任务拆解、子代理/规划器执行 | 反向绑定 Core |
| **Plugin Runtime** | 插件加载、握手与注册、生命周期管理、能力调用、隔离与异常收敛 | 直接嵌入 Core 内存模型 |

---

## 三、根目录结构

```
AstrCode/
├── crates/           # Rust 核心库（多 crate 架构）
├── frontend/         # React + TypeScript 前端
├── src-tauri/        # Tauri 桌面端薄壳
├── scripts/          # 构建脚本
├── .github/          # GitHub Actions 配置
├── Cargo.toml        # Workspace 配置
├── CLAUDE.md         # Claude Code 项目指引
├── AGENTS.md         # AI Agent 配置文件
└── deny.toml         # cargo deny 依赖检查配置
```

### 目录边界原则

| 目录 | 边界约束 |
|------|----------|
| `core/` | 不依赖 UI |
| `contracts/` (现为 `server/dto.rs`) | 不放业务逻辑 |
| `server/` | 不直接持有前端状态 |
| `src-tauri/` | 不承载业务真源 |

---

## 四、crates/ - Rust 核心库

采用分层架构设计，依赖方向为：`server → agent → core`，`tools` 独立于 `agent`。

### 架构演进方向

```
当前状态                              目标状态
────────                              ────────
crates/core/                    →    crates/core/         # 纯领域核心
crates/agent/                   →    crates/runtime/      # agent/plugin runtime
crates/server/                  →    crates/server/       # API 与流式输出
                                     crates/storage/      # event log / projection
                                     crates/plugins/      # 内置插件与 SDK glue
```

### crates/core/

**职责**：纯领域类型定义，无外部依赖，是整个系统的类型基石。

> 对应架构设计中的 "Core Kernel" 的核心类型部分。

| 文件 | 功能 | 架构映射 |
|------|------|----------|
| `lib.rs` | 模块导出入口 | — |
| `action.rs` | LLM 消息、响应、工具调用请求/结果等核心类型 | Capability 调用模型 |
| `event.rs` | `AgentEvent` 事件枚举、`Phase` 阶段状态 | Event 对象模型 |
| `tool.rs` | `Tool` trait、`ToolContext`、`SessionId` | Capability 抽象 |
| `cancel.rs` | `CancelToken` 取消令牌实现 | 调用模型 - Cancel |
| `error.rs` | `AstrError` 错误类型定义 | 错误模型 |
| `tests/` | 集成测试 | — |

### crates/agent/

**职责**：会话生命周期管理、JSONL 日志持久化、事件广播、配置管理。

> 当前承载了 "Agent Runtime" 和部分 "Storage" 职责，后续可拆分。

```
agent/
├── service/           # AgentService 门面 - Server 与 Core 的协调层
│   ├── mod.rs         # 门面入口
│   ├── config_ops.rs  # 配置操作
│   ├── session_ops.rs # 会话操作
│   ├── turn_ops.rs    # Turn 操作
│   ├── replay.rs      # SSE 事件回放
│   ├── session_state.rs # 会话状态管理
│   ├── types.rs       # 服务层类型
│   └── support.rs     # 支持工具
├── agent_loop/        # Agent 循环 - Runtime 核心
│   ├── llm_cycle.rs   # LLM 调用循环
│   ├── tool_cycle.rs  # 工具执行循环
│   └── turn_runner.rs # 单轮对话执行器
├── event_log/         # 事件存储 - Storage 层
│   ├── paths.rs       # 路径计算
│   ├── query.rs       # 查询逻辑
│   └── store.rs       # JSONL 存储
├── prompt/            # Prompt 构建 - Context Provider
│   ├── composer.rs    # 组装器
│   └── contributors/  # 各种贡献者
└── llm/               # LLM 适配 - Runtime 实现
```

**关键文件说明**：

| 文件/目录 | 功能 | 架构映射 |
|-----------|------|----------|
| `service/mod.rs` | `AgentService` 门面 | Server ↔ Core 协调 |
| `agent_loop/` | Agent 循环核心 | Agent Runtime |
| `event_log/` | 事件存储 | EventStore / Projection |
| `projection.rs` | 事件投影 | Projection 对象模型 |
| `tool_registry.rs` | 工具注册表 | ToolRegistry / CapabilityRegistry |
| `provider_factory.rs` | LLM 提供者工厂 | Runtime 可替换点 |
| `prompt/contributors/` | Prompt 贡献者 | Context Provider (插件化候选) |

### crates/server/

**职责**：Axum 本地 HTTP/SSE 服务器，**唯一业务入口**。

> 对应架构设计中的 "Server" 层，是 AstrCode 的真源入口。

| 文件 | 功能 | 架构映射 |
|------|------|----------|
| `main.rs` | 服务器入口，所有路由和处理器 | API 层 |
| `dto.rs` | HTTP/SSE DTO 定义 | Protocol / Contracts |

**API 路由**：

| 路由 | 方法 | 功能 |
|------|------|------|
| `/api/auth/exchange` | POST | Token 认证交换 |
| `/api/sessions` | GET/POST | 会话列表/创建 |
| `/api/sessions/:id/messages` | GET | 获取会话消息快照 |
| `/api/sessions/:id/prompts` | POST | 提交 Prompt |
| `/api/sessions/:id/interrupt` | POST | 中断会话 |
| `/api/sessions/:id/events` | GET | SSE 事件流 |
| `/api/sessions/:id` | DELETE | 删除会话 |
| `/api/projects` | DELETE | 删除项目 |
| `/api/config` | GET | 获取配置 |
| `/api/config/active-selection` | POST | 保存活跃配置 |
| `/api/models/current` | GET | 获取当前模型 |
| `/api/models` | GET | 列出所有可用模型 |
| `/api/models/test` | POST | 测试模型连接 |

### crates/tools/

**职责**：工具实现集合，不依赖 agent crate。

> 对应架构设计中的 "Built-in capabilities"，后续可插件化。

| 文件 | 功能 | 架构映射 |
|------|------|----------|
| `tools/shell.rs` | Shell 命令执行 | Tool (Capability) |
| `tools/read_file.rs` | 文件读取 | Tool (Capability) |
| `tools/write_file.rs` | 文件写入 | Tool (Capability) |
| `tools/edit_file.rs` | 文件编辑 | Tool (Capability) |
| `tools/list_dir.rs` | 目录列表 | Tool (Capability) |
| `tools/find_files.rs` | 文件查找 | Tool (Capability) |
| `tools/grep.rs` | 内容搜索 | Tool (Capability) |

---

## 五、src-tauri/ - Tauri 桌面端

**职责**：Tauri **薄壳**，仅承担宿主职责。

> 对应架构设计中的 "Tauri 边界" 约束。

| 文件 | 功能 | 边界约束 |
|------|------|----------|
| `src/main.rs` | Sidecar 启动、bootstrap 注入、退出清理 | 不承载业务逻辑 |
| `src/commands.rs` | 窗口控制、目录选择、配置编辑器打开 | 仅宿主能力桥接 |
| `src/paths.rs` | 路径计算工具 | 无业务语义 |

**Tauri 只负责**：
- 启动/关闭 sidecar server
- 窗口控制（最小化、最大化、关闭）
- 系统集成（目录选择对话框）
- 桌面能力桥接

**关键流程**：
1. 启动时 spawn `astrcode-server` sidecar
2. 等待 `run.json` 就绪（含 port/token）
3. 注入 `window.__ASTRCODE_BOOTSTRAP__` 到前端
4. 退出时清理 sidecar 进程

---

## 六、frontend/ - React 前端

**职责**：React + TypeScript + Vite UI，桌面端和浏览器端共用。

> 对应架构设计中的 "Frontends" 层，只做展示与交互，不持有真状态。

### 目录结构

```
frontend/src/
├── App.tsx              # 应用入口
├── main.tsx             # React 挂载点
├── types.ts             # TypeScript 类型定义
├── components/          # React 组件 - 交互呈现
│   ├── Chat/            # 聊天界面
│   ├── Sidebar/         # 侧边栏
│   └── Settings/        # 设置弹窗
├── hooks/               # React Hooks - 事件订阅
│   ├── useAgent.ts      # Agent 通信 Hook
│   └── useProjects.ts   # 项目管理 Hook
├── lib/                 # 工具库
│   ├── agentEvent.ts    # 事件规范化
│   ├── serverAuth.ts    # 服务端认证
│   ├── hostBridge.ts    # 宿主桥接抽象（UI 可替换关键点）
│   └── sessionMessages.ts # 会话消息处理
└── utils/               # 通用工具
```

### 前端边界

| 负责 | 不负责 |
|------|--------|
| 输入输出 | 会话真状态 |
| 交互呈现 | 工具调度 |
| 事件订阅 | 插件管理 |
| 局部缓存与乐观展示 | 核心业务决策 |

### 双端统一

前端通过 `hostBridge.ts` 抽象桌面端和浏览器端差异：
- **桌面端**：通过 Tauri API 调用原生功能
- **浏览器端**：部分功能不可用（如目录选择）

---

## 七、核心对象模型

### Session

表示一次工作会话，是平台的一级对象。

```rust
struct Session {
    id: SessionId,
    title: String,
    workspace_root: PathBuf,
    created_at: DateTime,
    updated_at: DateTime,
    runtime_profile: String,
    status: SessionStatus,
}
```

### Event

事件是**唯一可追溯事实**。所有状态变化都应可由事件重建。

**典型事件**：
- `SessionStart` - 会话开始
- `UserMessage` - 用户消息
- `AssistantDelta` - AI 增量输出
- `AssistantFinal` - AI 最终消息
- `ToolCall` / `ToolResult` - 工具调用
- `TurnDone` - 轮次结束
- `Error` - 错误

### Projection

Projection 是事件的投影结果，用于高效读取。

**典型投影**：
- `SessionListProjection` - 会话列表
- `ConversationProjection` - 对话状态
- `ToolHistoryProjection` - 工具调用历史

---

## 八、数据流架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        Frontend (React)                         │
│  ┌──────────┐    ┌──────────┐    ┌──────────────────────────┐  │
│  │ Sidebar  │    │   Chat   │    │       useAgent Hook      │  │
│  └────┬─────┘    └────┬─────┘    └────────────┬─────────────┘  │
│       │               │                       │                 │
│       └───────────────┴───────────────────────┘                 │
│                           │ HTTP/SSE                            │
│                           │ (协议边界)                           │
└───────────────────────────┼─────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    crates/server (Axum)                         │
│                    ★ Server is the Truth ★                      │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                    AppState                               │  │
│  │    ┌─────────────┐    ┌──────────────────────────────┐   │  │
│  │    │ AgentService│    │      ToolRegistry            │   │  │
│  │    └──────┬──────┘    └──────────────────────────────┘   │  │
│  └───────────┼──────────────────────────────────────────────┘  │
└──────────────┼──────────────────────────────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────────────────────────────┐
│                    crates/agent (Runtime)                        │
│  ┌───────────────┐    ┌───────────────┐    ┌────────────────┐  │
│  │  AgentLoop    │◄───│ ProviderFactory│    │   EventLog     │  │
│  │  (可替换)      │    │  (可替换)      │    │  (append-only) │  │
│  └───────┬───────┘    └───────────────┘    └────────────────┘  │
│          │                                                      │
│          ▼                                                      │
│  ┌───────────────┐    ┌───────────────┐                        │
│  │   LLM API     │    │    Tools      │                        │
│  │ (Anthropic/   │    │ (插件化候选)   │                        │
│  │  OpenAI)      │    │               │                        │
│  └───────────────┘    └───────────────┘                        │
└─────────────────────────────────────────────────────────────────┘
```

---

## 九、会话持久化模型

- **存储位置**：`~/.astrcode/sessions/session-*.jsonl`
- **格式**：append-only `StoredEvent { storage_seq, event }`
- **storage_seq**：由会话 writer 独占分配，保证单调递增
- **SSE 事件 ID**：`{storage_seq}.{subindex}` 格式

### 存储原则

| 原则 | 说明 |
|------|------|
| 文件系统优先 | 本地文件系统作为持久化真源，保证可调试、可迁移、可恢复、无数据库强依赖 |
| 事件追加式写入 | 会话事件采用 append-only 方式保存，读取时通过 projection 恢复当前状态 |
| 删除策略 | 删除优先做物理删除或明确 tombstone，避免隐式悬挂状态 |

---

## 十、协议设计

### 基础消息类型

| 类型 | 用途 |
|------|------|
| `Initialize` | 握手初始化 |
| `Invoke` | 能力调用 |
| `Result` | 调用结果 |
| `Event` | 事件通知 |
| `Cancel` | 取消请求 |

### DTO 边界

> DTO 定义在 `crates/server/src/dto.rs`，不放业务逻辑。

**核心 DTO**：
- `AgentEventEnvelope` - SSE 事件信封
- `SessionListItem` - 会话列表项
- `SessionMessageDto` - 消息 DTO
- `PromptRequest/Response` - Prompt 提交
- `ConfigView` / `ProfileView` - 配置视图
- `ModelOptionDto` / `CurrentModelInfoDto` - 模型信息

### 错误模型

统一错误结构：
```rust
struct ApiError {
    code: String,      // 稳定字符串，非语言绑定异常类型
    message: String,
    retryable: bool,
    details: Option<Value>,
}
```

---

## 十一、构建命令

```bash
# 开发模式（Tauri）
cargo tauri dev

# 仅浏览器端本地服务器
cargo run -p astrcode-server

# 生产构建
cargo tauri build

# 运行所有测试
cargo test --workspace

# 工作区检查
cargo check --workspace

# 依赖边界检查
cargo deny check bans

# 前端开发
cd frontend && npm run dev

# 前端类型检查
cd frontend && npm run typecheck
```

---

## 十二、配置文件

| 文件 | 位置 | 用途 |
|------|------|------|
| `config.json` | `~/.astrcode/` | API 密钥、Profile 配置 |
| `run.json` | `~/.astrcode/` | 运行时信息（port/token/pid） |
| `sessions/*.jsonl` | `~/.astrcode/sessions/` | 会话持久化 |

---

## 十三、架构演进路线

### 当前状态 → 目标状态

| 当前 | 目标 | 演进方向 |
|------|------|----------|
| `crates/agent/` 承载 Runtime + Storage | 拆分为 `runtime/` + `storage/` | 职责分离 |
| DTO 在 `server/dto.rs` | 独立 `contracts/` crate | 协议边界清晰化 |
| Tools 硬编码在 `crates/tools/` | 插件化，通过 Plugin Runtime 加载 | 插件优先 |
| 单一 Agent Loop | 可替换 Runtime（Native / LangChain 等） | Runtime 可替换 |
| `provider_factory.rs` 硬编码 LLM | Runtime 层适配器 | 框架解绑 |

### 演进原则

1. 新能力优先作为插件或 runtime 扩展引入
2. 新前端不得绕过 Server 直接写核心状态
3. 新协议字段尽量追加，不随意破坏兼容
4. Core 尽量只增加抽象，不增加产品耦合
5. 能用 DTO 表达的对象，不直接跨边界传运行时实例

---

## 十四、非目标

当前阶段，AstrCode **暂不追求**：

- 分布式多机调度
- 公网多租户安全模型
- 复杂数据库优先架构
- 与某单一 Agent 框架深度绑定
- 大而全的插件市场模型

**先保证本地单机架构闭环清晰。**

---

*报告结束*
