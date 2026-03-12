# AstrCode 项目结构报告

> 生成时间：2026-03-12

## 项目概述

AstrCode 是一个基于 AI 的代码助手应用，支持桌面端（Tauri）和浏览器端双模式运行。项目采用 Rust 后端 + React 前端的架构，通过 HTTP/SSE 进行前后端通信。

---

## 根目录结构

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

---

## crates/ - Rust 核心库

采用分层架构设计，依赖方向为：`server → agent → core`，`tools` 独立于 `agent`。

### crates/core/

**职责**：纯领域类型定义，无外部依赖，是整个系统的类型基石。

| 文件 | 功能 |
|------|------|
| `lib.rs` | 模块导出入口 |
| `action.rs` | LLM 消息、响应、工具调用请求/结果等核心类型 |
| `event.rs` | `AgentEvent` 事件枚举、`Phase` 阶段状态、`ToolCallEventResult` |
| `tool.rs` | `Tool` trait、`ToolContext`、`SessionId` 类型定义 |
| `cancel.rs` | `CancelToken` 取消令牌实现 |
| `agent_loop/` | Agent 循环核心逻辑（状态机） |
| `event_log/` | 事件日志存储抽象 |
| `llm/` | LLM 提供者抽象（Anthropic、OpenAI） |
| `prompt/` | Prompt 构建系统 |
| `runtime/` | Agent 运行时组装 |
| `tests/` | 集成测试 |

### crates/agent/

**职责**：会话生命周期管理、JSONL 日志持久化、事件广播、配置管理。

| 文件 | 功能 |
|------|------|
| `lib.rs` | 模块导出入口 |
| `service.rs` | `AgentService` - 核心服务，管理会话状态、广播、回放 |
| `agent_loop.rs` | Agent 循环封装，协调 LLM 调用和工具执行 |
| `agent_loop/llm_cycle.rs` | LLM 调用循环逻辑 |
| `agent_loop/tool_cycle.rs` | 工具执行循环逻辑 |
| `agent_loop/turn_runner.rs` | 单轮对话执行器 |
| `config.rs` | 配置文件加载/保存（`~/.astrcode/config.json`） |
| `event_log.rs` | 事件日志门面 |
| `event_log/paths.rs` | 会话文件路径计算 |
| `event_log/query.rs` | 会话查询逻辑 |
| `event_log/store.rs` | JSONL 存储实现 |
| `events.rs` | `StorageEvent` 存储层事件定义 |
| `projection.rs` | 事件投影，重建对话状态 |
| `tool_registry.rs` | 工具注册表（冻结后只读） |
| `provider_factory.rs` | LLM 提供者工厂 |
| `llm/anthropic.rs` | Anthropic API 适配 |
| `llm/openai.rs` | OpenAI API 适配 |
| `prompt/composer.rs` | Prompt 组装器 |
| `prompt/contributors/` | Prompt 贡献者（identity、environment、skill 等） |

### crates/contracts/

**职责**：HTTP/SSE 数据传输对象（DTO），定义前后端通信协议。

| 文件 | 功能 |
|------|------|
| `lib.rs` | 所有 DTO 定义 |
| - | `AgentEventEnvelope` - SSE 事件信封 |
| - | `SessionListItem` - 会话列表项 |
| - | `SessionMessageDto` - 消息 DTO（User/Assistant/ToolCall） |
| - | `PromptRequest/Response` - Prompt 提交 |
| - | `ConfigView` - 配置视图 |
| - | `ModelOptionDto` - 模型选项 |

### crates/server/

**职责**：Axum 本地 HTTP/SSE 服务器，唯一业务入口。

| 文件 | 功能 |
|------|------|
| `main.rs` | 服务器入口，包含所有路由和处理器 |

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
| `/api/projects` | DELETE | 删除项目（及其所有会话） |
| `/api/config` | GET | 获取配置 |
| `/api/config/active-selection` | POST | 保存活跃配置 |
| `/api/models/current` | GET | 获取当前模型 |
| `/api/models` | GET | 列出所有可用模型 |
| `/api/models/test` | POST | 测试模型连接 |

### crates/tools/

**职责**：工具实现集合，不依赖 agent crate。

| 文件/目录 | 功能 |
|-----------|------|
| `lib.rs` | 工具模块导出 |
| `tools/shell.rs` | Shell 命令执行工具 |
| `tools/read_file.rs` | 文件读取工具 |
| `tools/write_file.rs` | 文件写入工具 |
| `tools/edit_file.rs` | 文件编辑工具 |
| `tools/list_dir.rs` | 目录列表工具 |
| `tools/find_files.rs` | 文件查找工具（glob 模式） |
| `tools/grep.rs` | 内容搜索工具 |
| `tools/fs_common.rs` | 文件系统通用工具函数 |
| `test_support.rs` | 测试支持 |

### crates/ipc/

**职责**：IPC 通信协议定义（预留，当前主要使用 contracts）。

| 文件 | 功能 |
|------|------|
| `lib.rs` | IPC 协议类型定义 |

---

## src-tauri/ - Tauri 桌面端

**职责**：Tauri 薄壳，负责 sidecar 管理、窗口控制、宿主 GUI 桥接。

| 文件 | 功能 |
|------|------|
| `src/main.rs` | Tauri 应用入口，sidecar 启动、bootstrap 注入、退出清理 |
| `src/commands.rs` | Tauri 命令定义（窗口控制、目录选择等） |
| `src/paths.rs` | 路径计算工具 |
| `src/handle.rs` | Agent 句柄管理 |
| `src/handle/session_service.rs` | 会话服务桥接 |
| `src/handle/model_service.rs` | 模型服务桥接 |
| `src/handle/prompt_service.rs` | Prompt 服务桥接 |
| `src/handle/ipc/event_bridge.rs` | 事件桥接 |
| `src/handle/presentation/` | 视图模型转换 |

**关键流程**：
1. 启动时 spawn `astrcode-server` sidecar
2. 等待 `run.json` 就绪（含 port/token）
3. 注入 `window.__ASTRCODE_BOOTSTRAP__` 到前端
4. 退出时清理 sidecar 进程

---

## frontend/ - React 前端

**职责**：React + TypeScript + Vite UI，桌面端和浏览器端共用。

### 目录结构

```
frontend/src/
├── App.tsx              # 应用入口，状态管理
├── main.tsx             # React 挂载点
├── types.ts             # TypeScript 类型定义
├── index.css            # 全局样式
├── components/          # React 组件
│   ├── Chat/            # 聊天界面
│   ├── Sidebar/         # 侧边栏
│   ├── Settings/        # 设置弹窗
│   └── NewProjectModal  # 新建项目弹窗
├── hooks/               # React Hooks
│   └── useAgent.ts      # Agent 通信 Hook
├── lib/                 # 工具库
│   ├── agentEvent.ts    # 事件规范化
│   ├── serverAuth.ts    # 服务端认证
│   ├── hostBridge.ts    # 宿主桥接抽象
│   ├── tauri.ts         # Tauri API 封装
│   └── turnRouting.ts   # Turn ID 路由
└── utils/               # 通用工具
    └── uuid.ts          # UUID 生成
```

### 核心组件

| 组件 | 功能 |
|------|------|
| `App.tsx` | 应用入口，全局状态管理（useReducer），事件处理 |
| `Chat/index.tsx` | 聊天主界面 |
| `Chat/MessageList.tsx` | 消息列表渲染 |
| `Chat/InputBar.tsx` | 输入栏 |
| `Chat/AssistantMessage.tsx` | AI 消息渲染 |
| `Chat/UserMessage.tsx` | 用户消息渲染 |
| `Chat/ToolCallBlock.tsx` | 工具调用展示 |
| `Chat/ModelSelector.tsx` | 模型选择器 |
| `Sidebar/index.tsx` | 项目/会话列表 |
| `Sidebar/SessionItem.tsx` | 会话项 |
| `Sidebar/ProjectItem.tsx` | 项目项 |
| `Settings/SettingsModal.tsx` | 设置弹窗 |

### 核心 Hooks

| Hook | 功能 |
|------|------|
| `useAgent.ts` | 统一的 fetch + EventSource 客户端，封装所有 API 调用 |

**useAgent 提供的方法**：
- `createSession` - 创建会话
- `listSessionsWithMeta` - 获取会话列表
- `loadSession` - 加载会话消息快照
- `connectSession` - 连接 SSE 事件流
- `disconnectSession` - 断开连接
- `submitPrompt` - 提交 Prompt
- `interrupt` - 中断会话
- `deleteSession` / `deleteProject` - 删除操作
- `getConfig` / `saveActiveSelection` - 配置管理
- `getCurrentModel` / `listAvailableModels` / `setModel` - 模型管理
- `testConnection` - 测试连接
- `openConfigInEditor` / `selectDirectory` - 宿主功能

---

## 数据流架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        Frontend (React)                         │
│  ┌──────────┐    ┌──────────┐    ┌──────────────────────────┐  │
│  │ Sidebar  │    │   Chat   │    │       useAgent Hook      │  │
│  └────┬─────┘    └────┬─────┘    └────────────┬─────────────┘  │
│       │               │                       │                 │
│       └───────────────┴───────────────────────┘                 │
│                           │ HTTP/SSE                            │
└───────────────────────────┼─────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    crates/server (Axum)                         │
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
│                    crates/agent                                  │
│  ┌───────────────┐    ┌───────────────┐    ┌────────────────┐  │
│  │  AgentLoop    │◄───│ ProviderFactory│    │   EventLog     │  │
│  └───────┬───────┘    └───────────────┘    └────────────────┘  │
│          │                                                      │
│          ▼                                                      │
│  ┌───────────────┐    ┌───────────────┐                        │
│  │   LLM API     │    │    Tools      │                        │
│  │ (Anthropic/   │    │ (Shell/Edit/  │                        │
│  │  OpenAI)      │    │  Read/...)    │                        │
│  └───────────────┘    └───────────────┘                        │
└─────────────────────────────────────────────────────────────────┘
```

---

## 会话持久化模型

- **存储位置**：`~/.astrcode/sessions/session-*.jsonl`
- **格式**：append-only `StoredEvent { storage_seq, event }`
- **storage_seq**：由会话 writer 独占分配，保证单调递增
- **SSE 事件 ID**：`{storage_seq}.{subindex}` 格式

**事件类型（StorageEvent）**：
- `SessionStart` - 会话开始
- `UserMessage` - 用户消息
- `AssistantDelta` - AI 增量输出
- `AssistantFinal` - AI 最终消息
- `ToolCall` / `ToolResult` - 工具调用
- `TurnDone` - 轮次结束
- `Error` - 错误

---

## 关键设计决策

### Server Is The Truth
所有会话、配置、模型、事件流业务入口只通过 `crates/server` 暴露的 HTTP/SSE API。前端和 Tauri 都不得直接调用 agent。

### Tool Error 语义
- `Err(anyhow::Error)` → 系统级失败（IO 错误、参数解析失败、取消）
- `ToolExecutionResult { ok: false }` → 工具级拒绝（安全策略、需用户确认）

### 双端统一
前端通过 `hostBridge.ts` 抽象桌面端和浏览器端差异：
- 桌面端：通过 Tauri API 调用原生功能
- 浏览器端：部分功能不可用（如目录选择）

---

## 构建命令

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

## 配置文件

| 文件 | 位置 | 用途 |
|------|------|------|
| `config.json` | `~/.astrcode/` | API 密钥、Profile 配置 |
| `run.json` | `~/.astrcode/` | 运行时信息（port/token/pid） |
| `sessions/*.jsonl` | `~/.astrcode/sessions/` | 会话持久化 |

---

*报告结束*
