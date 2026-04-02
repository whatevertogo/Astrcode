# Frontend Architecture

## Overview

React 18 + TypeScript 单页应用，无客户端路由.通过 SSE 双通道与 `crates/server` 实时通信.

### Technology Stack

| 层 | 技术 |
|---|------|
| UI | React 18 (Hooks), CSS Modules |
| 状态 | `useReducer` + `store/reducer.ts` |
| Markdown | `react-markdown` + `remark-gfm` |
| HTTP | `fetch` (自定义客户端封装) |
| SSE | 原生 `EventSource` 替代品 (基于 `Response.body.getReader()`) |
| 桌面壳 | Tauri (可选, 有浏览器回退模式) |

---

## Entry Point

```
frontend/src/main.tsx
  → RootErrorBoundary (Class Component)
    → App.tsx (主编排器)
```

**`App.tsx` 职责**:
- 管理 `sidebar`(左) + `chat`(右) 两栏布局
- `useReducer` 状态树
- 会话生命周期: 创建/加载/切换/删除/分支检测
- `Phase` 状态: `idle | thinking | callingTool | streaming | interrupted | done`
- `streamGenerationRef` — 防止会话切换时的竞态条件

---

## State Management (`store/`)

**`store/reducer.ts`** (~500 行) — 中央 `useReducer`:

| Action 类别 | 示例 |
|------------|------|
| 会话生命周期 | `INITIALIZE`, `SET_ACTIVE`, `ADD_SESSION`, `DELETE_SESSION` |
| 消息流式 | `APPEND_DELTA`, `APPEND_REASONING_DELTA`, `FINALIZE_ASSISTANT`, `END_STREAMING` |
| 工具事件 | `APPEND_TOOL_CALL_DELTA`, `UPDATE_TOOL_CALL`, `ADD_MESSAGE` |
| 阶段 | `SET_PHASE` |
| 批量替换 | `REPLACE_SESSION_MESSAGES` (用于会话快照加载) |

**State 形状**:
```ts
interface AppState {
  projects: Project[];           // 按工作目录分组的会话
  activeProjectId: string | null;
  activeSessionId: string | null;
  phase: Phase;
}
```

**工具函数**: `convertSessionMessages()` — 将 API 返回的快照消息转换为前端 `Message` 类型

---

## API Integration Layer (`lib/api/`)

统一 HTTP 客户端，自动注入 `x-astrcode-token`:

| 模块 | 端点 | 方法 |
|------|------|------|
| `lib/api/sessions.ts` | `/api/sessions` | CRUD + prompt 提交 + 中断 |
| `lib/api/config.ts` | `/api/config` | 查询/保存 active selection |
| `lib/api/models.ts` | `/api/models/*` | 模型列表/当前模型/连接测试 |

**核心方法签名**:
```ts
createSession(workingDir: string) → Promise<string>
listSessionsWithMeta() → Promise<SessionMeta[]>
loadSession(sessionId: string) → Promise<{ messages: Message[], cursor: string }>
submitPrompt(sessionId: string, text: string) → Promise<void>
interruptSession(sessionId: string) → Promise<void>
```

---

## SSE (Server-Sent Events)

前端维护两条 SSE 通道:

### 1. Agent Events (Per-Session)

- URL: `/api/sessions/:id/events?afterEventId=<cursor>`
- 用途: 单会话 agent 事件流 (思考/工具/消息)
- 重连策略: 指数退底 (500ms-5000ms)
- 协议版本校验: 丢弃 `protocolVersion !== 1` 的事件

### 2. Session Catalog Events (Global)

- URL: `/api/session-events`
- 用途: 全局会话创建/删除/分支事件
- 断线后触发全量会话同步

**`lib/sse/consumer.ts`**: 通用 SSE 流解析器，处理 `id:`/`event:`/`data:` 帧.

**`lib/agentEvent.ts`**: 协议 v1 规范化器 — 校验 `protocolVersion`, 驼峰/蛇形转换, 事件分发:
- `sessionStarted`, `userMessage`, `phaseChanged`
- `modelDelta`, `thinkingDelta`, `assistantMessage`
- `toolCallStart`, `toolCallDelta`, `toolCallResult`
- `turnDone`, `error`

---

## Custom Hooks (`hooks/`)

| Hook | 职责 |
|------|------|
| `useAgent()` | 主 API 协调: CRUD + SSE 生命周期 (连接/断开/重连), generation-based 竞态保护 |
| `useAgentEventHandler()` | SSE 事件 → dispatch action 映射, phase 管理, turn 路由 |
| `useSessionCatalogEvents()` | 全局会话目录 SSE, 断线重连触发全量同步 |
| `useSidebarResize()` | 可拖拽侧栏 (220px-420px, localStorage 持久化) |

**关键设计模式**:
- `streamGenerationRef` / `sessionActivationGenerationRef` — 防止会话切换时的陈旧 SSE 事件污染
- `turnSessionMapRef` — 将 turnId 映射到 sessionId，处理分支场景
- `startTransition()` — 流式更新不阻塞高优先级渲染

---

## Component Tree

```
App
  |-- Sidebar (可拖拽)
  |     |-- ProjectItem (可展开, 右键菜单)
  |     |     |-- SessionItem (右键菜单)
  |     |-- NewProjectModal
  |
  |-- Chat
  |     |-- TopBar
  |     |     |-- ModelSelector
  |     |-- MessageList
  |     |     |-- MessageBoundary (每消息的错误边界)
  |     |     |-- UserMessage
  |     |     |-- AssistantMessage (思考块 + ReactMarkdown 渲染)
  |     |     |-- ToolCallBlock (diff 视图 / 终端视图 / 原始输出)
  |     |-- InputBar
  |
  |-- SettingsModal
```

---

## Tool Rendering (`components/Chat/ ToolCallBlock.tsx` + `lib/`)

`ToolCallBlock` 根据 metadata 选择三种渲染模式:

### 1. Diff 视图 (writeFile/editFile)
- 触发条件: `metadata.diff.patch` 存在
- 展示: 变更类型、路径、字节数、+/- 行数
- 代码高亮: `classifyToolDiffLine(line)` → `'meta' | 'header' | 'add' | 'remove' | 'note' | 'context'`
- 源码: `lib/toolDiff.ts`

### 2. 终端视图 (shell)
- 触发条件: `metadata.display.kind === 'terminal'`
- 展示: 命令、cwd、shell、exit code、stdout/stderr 分段(带错误样式)
- 源码: `lib/toolDisplay.ts`

### 3. 原始输出
- 触发条件: 无 metadata 驱动
- 展示: `<pre>` 纯文本块

---

## Auth Flow (`lib/serverAuth.ts`)

```
1. 检测运行模式: Tauri Desktop / 浏览器 Dev (port 5173) / 注入 __ASTRCODE_BOOTSTRAP__
2. POST /api/auth/exchange — 用 bootstrap token 交换 session token
3. Session token 通过 x-astrcode-token 头注入所有后续 API 请求
4. ensureServerSession() — 每次 API 请求前调用, token 过期时自动刷新
```

---

## Key Files

| 文件 | 职责 |
|------|------|
| `src/App.tsx` | 主编排器, 状态树, 会话生命周期 |
| `src/store/reducer.ts` | 中央状态 reducer |
| `src/lib/api/client.ts` | HTTP fetch 封装 |
| `src/lib/api/sessions.ts` | 会话 API |
| `src/lib/agentEvent.ts` | SSE 事件协议 v1 规范化器 |
| `src/lib/sse/consumer.ts` | SSE 流解析器 |
| `src/lib/serverAuth.ts` | Bootstrap 认证流程 |
| `src/lib/toolDiff.ts` | Diff 元数据提取/行分类 |
| `src/lib/toolDisplay.ts` | Shell/终端元数据提取 |
| `src/components/Chat/ToolCallBlock.tsx` | 工具卡片组件 |
| `src/hooks/useAgentEventHandler.ts` | SSE 事件 → reducer dispatch |
| `src/hooks/useAgent.ts` | 主 API 协调 hook |
