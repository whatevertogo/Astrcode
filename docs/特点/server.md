# Server-First 前后端分离架构

Astrcode 采用 **Server Is The Truth** 原则：所有业务逻辑（会话管理、LLM 调用、工具执行、治理决策）都在后端 HTTP/SSE 服务器中完成，前端和 Tauri 外壳不绕过服务器直接调用运行时。

## 架构总览

三个部署形态共用同一个后端：

```
┌─────────────────────────────────────────────────────────┐
│                     Rust 后端服务器                      │
│                 (crates/server, Axum)                    │
│                                                         │
│  ┌──────────┐  ┌──────────────┐  ┌───────────────────┐ │
│  │ REST API │  │  SSE 流式端点 │  │  静态资源托管     │ │
│  └────┬─────┘  └──────┬───────┘  └───────────────────┘ │
│       │               │                                  │
│  ┌────┴───────────────┴─────────────────────────────┐   │
│  │        Application Layer (use cases)             │   │
│  │  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │   │
│  │  │ Session  │  │  Agent   │  │  Governance   │  │   │
│  │  │ Runtime  │  │  Exec    │  │  Mode System  │  │   │
│  │  └──────────┘  └──────────┘  └───────────────┘  │   │
│  └──────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────┐   │
│  │  Event Log (JSONL) + CQRS Projections            │   │
│  └──────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
          ▲                              ▲              ▲
          │ HTTP/SSE                     │ HTTP/SSE     │ HTTP/SSE
   ┌──────┴──────┐              ┌────────┴──────┐  ┌───┴────┐
   │   Tauri     │              │   Browser     │  │  CLI   │
   │   Desktop   │              │   Dev Mode    │  │ Client │
   │  (WebView)  │              │  (Vite :5173) │  │        │
   └─────────────┘              └───────────────┘  └────────┘
```

## 后端（Rust / Axum）

### 启动流程

服务器启动时（`crates/server/src/main.rs`）：

1. Bootstrap 运行时（加载插件、初始化 LLM provider、MCP server）
2. 绑定 `127.0.0.1:0`（随机端口）
3. 生成 bootstrap token（32 字节 hex，24 小时 TTL）
4. 写入 `~/.astrcode/run.json`（port、token、pid、startedAt、expiresAtMs）
5. 在 stdout 输出结构化 JSON ready 事件
6. 启动 HTTP 服务，监听优雅关闭信号

### API 路由

路由定义在 `crates/server/src/http/routes/mod.rs`：

**会话管理：**

| 方法 | 路径 | 用途 |
|------|------|------|
| POST | `/api/sessions` | 创建会话 |
| GET | `/api/sessions` | 列出会话 |
| DELETE | `/api/sessions/{id}` | 删除会话 |
| POST | `/api/sessions/{id}/fork` | 分叉会话 |
| GET | `/api/session-events` | SSE 会话目录事件流 |

**对话交互：**

| 方法 | 路径 | 用途 |
|------|------|------|
| POST | `/api/sessions/{id}/prompts` | 提交用户 prompt |
| POST | `/api/sessions/{id}/compact` | 触发上下文压缩 |
| POST | `/api/sessions/{id}/interrupt` | 中断执行 |
| GET/POST | `/api/sessions/{id}/mode` | 查询/切换治理模式 |
| GET | `/api/v1/conversation/sessions/{id}/snapshot` | 获取对话快照 |
| GET | `/api/v1/conversation/sessions/{id}/stream` | SSE 对话增量流 |

**配置与模型：**

| 方法 | 路径 | 用途 |
|------|------|------|
| GET/POST | `/api/config` | 获取/更新配置 |
| POST | `/api/config/reload` | 热重载配置 |
| GET | `/api/models` | 列出可用模型 |
| GET | `/api/modes` | 列出治理模式 |

**Agent 编排：**

| 方法 | 路径 | 用途 |
|------|------|------|
| POST | `/api/v1/agents/{id}/execute` | 执行 agent |
| GET | `/api/v1/sessions/{id}/subruns/{sub_run_id}` | 子运行状态 |
| POST | `/api/v1/sessions/{id}/agents/{agent_id}/close` | 关闭 agent 树 |

### 认证

三级 token 体系（`crates/server/src/http/auth.rs`）：

```
Bootstrap Token (24h)
    │
    ▼  POST /api/auth/exchange
Session Token
    │
    ▼  x-astrcode-token header (或 SSE ?token= 参数)
Per-Request 认证
```

- **Desktop**：Tauri 通过 `window.__ASTRCODE_BOOTSTRAP__` 注入 token
- **Browser dev**：前端从 `GET /__astrcode__/run-info` 获取 token

## 通信协议

### REST + SSE，不使用 WebSocket

所有通信基于 HTTP。请求-响应操作走 REST JSON，实时推送走 SSE。

### 对话协议：Snapshot + Delta

前端获取对话状态分两步（`crates/protocol/src/http/conversation/v1.rs`）：

**第一步：获取快照**

```
GET /api/v1/conversation/sessions/{id}/snapshot
```

返回完整的对话状态：

```rust
pub struct ConversationSnapshotResponseDto {
    pub session_id: String,
    pub cursor: ConversationCursorDto,       // 游标（用于后续 SSE 续接）
    pub phase: PhaseDto,                      // 当前阶段
    pub control: ConversationControlStateDto, // 控制状态
    pub blocks: Vec<ConversationBlockDto>,    // 对话块列表
    pub child_summaries: Vec<...>,            // 子 agent 摘要
    pub slash_candidates: Vec<...>,           // slash 命令候选
    pub banner: Option<...>,                  // 顶部提示横幅
}
```

**第二步：订阅增量流**

```
GET /api/v1/conversation/sessions/{id}/stream
```

SSE 推送 `ConversationDeltaDto`，8 种增量类型：

| Delta 类型 | 用途 |
|-----------|------|
| `append_block` | 新增对话块（用户消息、助手回复、工具调用等） |
| `patch_block` | 增量更新已有块（markdown 追加、工具流式输出） |
| `complete_block` | 块完成（streaming -> complete/failed） |
| `update_control_state` | 阶段变更（thinking、idle 等） |
| `upsert_child_summary` | 子 agent 状态更新 |
| `remove_child_summary` | 子 agent 移除 |
| `set_banner` / `clear_banner` | 提示横幅 |
| `rehydrate_required` | 客户端需要重新获取快照 |

### 对话块类型

`ConversationBlockDto` 是前端渲染的基本单元：

| 块类型 | 说明 |
|--------|------|
| `User` | 用户消息 |
| `Assistant` | LLM 输出（支持流式追加） |
| `Thinking` | 扩展思考内容 |
| `Plan` | 会话计划引用 |
| `ToolCall` | 工具执行（含流式 stdout/stderr） |
| `Error` | 错误展示 |
| `SystemNote` | 系统备注（如 compact summary） |
| `ChildHandoff` | 子 agent 委派/归还通知 |

工具调用块的流式输出通过 `patch_block` 的 `AppendToolStream` 增量实现，支持 stdout 和 stderr 两个流。

### 游标续接

每个 SSE 事件携带 `id:` 字段（格式如 `"1.42"`，即 `{storage_seq}.{subindex}`）。断线重连时，客户端通过 `Last-Event-ID` header 或 `?cursor=` 参数发送上次游标，服务器只回放缺失的事件。

### 一次对话 turn 的完整流程

```
前端                              后端
 │                                │
 │  POST /prompts {text}          │
 │ ─────────────────────────────> │  追加 UserMessage 事件
 │  202 Accepted {turnId}         │  启动 turn 执行
 │ <───────────────────────────── │
 │                                │
 │  SSE: update_control_state     │  phase -> Thinking
 │ <────────────────────────────── │
 │  SSE: append_block Assistant   │  LLM 开始输出
 │ <────────────────────────────── │
 │  SSE: patch_block markdown     │  流式文本追加
 │ <────────────────────────────── │
 │  SSE: append_block ToolCall    │  工具调用开始
 │ <────────────────────────────── │
 │  SSE: patch_block tool_stream  │  工具 stdout/stderr 流
 │ <────────────────────────────── │
 │  SSE: complete_block           │  工具完成
 │ <────────────────────────────── │
 │  SSE: patch_block markdown     │  LLM 继续输出
 │ <────────────────────────────── │
 │  SSE: update_control_state     │  phase -> Idle
 │ <────────────────────────────── │
```

## 前端（React + TypeScript）

### 技术栈

- **框架**：React 18 + TypeScript
- **构建**：Vite
- **样式**：Tailwind CSS
- **状态管理**：原生 `useReducer`（无 Redux/Zustand）
- **SSE 消费**：手动 `ReadableStream` 解析（`frontend/src/lib/sse/consumer.ts`）

### 核心模块

```
frontend/src/
├── App.tsx                          # 根组件，useReducer 状态管理
├── types.ts                         # 共享 TypeScript 类型
├── store/reducer.ts                 # 中心状态 reducer
├── hooks/
│   ├── useAgent.ts                  # 核心编排 hook：对话流连接、API 调用
│   ├── app/
│   │   ├── useSessionCoordinator.ts # 会话生命周期管理
│   │   └── useComposerActions.ts    # 用户输入处理
│   └── useSessionCatalogEvents.ts   # 会话目录 SSE 订阅
├── lib/
│   ├── api/
│   │   ├── client.ts                # HTTP 客户端（auth header 注入）
│   │   ├── sessions.ts              # 会话 CRUD
│   │   ├── conversation.ts          # 对话快照/流解析 -> 前端 Message[]
│   │   ├── config.ts                # 配置 API
│   │   └── models.ts                # 模型 API
│   ├── sse/consumer.ts              # SSE 流解析器
│   ├── serverAuth.ts                # Bootstrap token 获取与交换
│   └── hostBridge.ts                # Desktop/Browser 能力抽象
```

### 状态同步

遵循 **Authoritative Server / Projected Client** 模型：

1. **快照全量 + 流式增量**：前端先获取完整快照，再订阅 SSE 增量
2. **客户端投影**：`applyConversationEnvelope()` 在前端将 delta 应用到本地状态，`projectConversationState()` 推导出扁平的 `Message[]` 数组用于渲染
3. **指纹去重**：`projectionSignature()` 基于内容哈希跳过冗余状态更新
4. **渲染批处理**：SSE 事件通过 `requestAnimationFrame` 批量应用，避免 React 渲染洪水
5. **自动重连**：指数退避（500ms 基础，5s 上限，3 次失败后放弃），使用 `Last-Event-ID` 续接

### HostBridge 抽象

`hostBridge.ts` 提供跨平台能力抽象：

- **Desktop**（Tauri）：通过 `invoke()` 调用 Tauri 命令（窗口控制、目录选择器）
- **Browser**：降级为 Web API（`window.open`、`<input type="file">`）

前端通过统一接口调用，不感知运行环境。

## Tauri 外壳

Tauri 是**纯薄壳**，不含业务逻辑（`src-tauri/src/main.rs`）。

### 职责

1. **单实例协调**：`DesktopInstanceCoordinator` 确保只有一个桌面实例运行
2. **Sidecar 启动**：启动 `astrcode-server` 作为子进程
3. **Bootstrap 注入**：将 token 注入 `window.__ASTRCODE_BOOTSTRAP__`
4. **窗口生命周期**：创建/管理主窗口

### 五个 Tauri 命令（`src-tauri/src/commands.rs`）

全部是 GUI 相关：

- `minimize_window` / `maximize_window` / `close_window` — 窗口控制
- `select_directory` — 原生目录选择器
- `open_config_in_editor` — 在系统编辑器中打开配置文件

### Sidecar 通信协议

```
1. Tauri 启动 astrcode-server 子进程
2. Server 在 stdout 输出 ready JSON：{"ready": true, "port": N, "pid": P}
3. Tauri 解析输出，轮询 HTTP 端口就绪
4. Server 写入 ~/.astrcode/run.json（供 browser dev 桥接）
5. 退出时 Tauri 释放子进程句柄，server 检测 stdin EOF 优雅关闭
```

## 协议层（crates/protocol）

`crates/protocol/` 定义所有前后端共享的 wire-format DTO：

```
crates/protocol/src/http/
├── auth.rs              # 认证交换 DTO
├── conversation/v1.rs   # 对话协议（快照 + delta DTO）
├── session.rs           # 会话管理 DTO
├── event.rs             # Agent 事件 DTO
├── session_event.rs     # 会话目录事件信封
├── config.rs            # 配置 DTO
├── model.rs             # 模型信息 DTO
├── composer.rs          # 输入补全 DTO
├── agent.rs             # Agent profile DTO
├── tool.rs              # 工具描述 DTO
├── runtime.rs           # 运行时状态/指标 DTO
└── terminal/            # CLI 终端投影 DTO
```

协议版本常量 `PROTOCOL_VERSION = 1`，包含在目录事件信封中。

## 设计决策与权衡

### 为什么选 REST + SSE 而非 WebSocket

- **SSE 天然支持游标续接**：每个事件有 `id`，断线重连只需发送 `Last-Event-ID`，服务端精确回放缺失事件
- **单向推送模型匹配实际需求**：对话场景是"客户端请求 -> 服务端长时流式响应"，不需要全双工
- **HTTP 生态友好**：负载均衡、代理、调试工具都原生支持 SSE，WebSocket 需要额外配置
- **事件溯源天然适配**：event log 中的 `storage_seq` 直接映射为 SSE 游标

### 为什么 Tauri 只做薄壳

- **部署一致性**：Desktop、Browser、CLI 三个入口走同一个 HTTP API，行为完全一致
- **独立演进**：前端可以不依赖 Tauri 版本独立更新，后端也可以独立迭代
- **测试简化**：后端测试只需要 HTTP 测试，不需要 Tauri WebView 环境

### 为什么前端状态管理用 useReducer

- **应用状态维度有限**：会话列表、当前会话、UI 阶段——不需要全局状态库的复杂度
- **服务端权威**：关键状态在后端 event log 中，前端只是投影，本地状态相对简单
- **避免间接层**：reducer 的 action 直接映射后端 DTO，调试路径清晰
