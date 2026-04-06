# Frontend Architecture

React 18 + TypeScript SPA，无客户端路由，通过 SSE 双通道与 `crates/server` 实时通信。

## Tech Stack

| 层 | 技术 |
|---|------|
| UI | React 18 (Hooks), CSS Modules + Tailwind CSS v4 |
| Markdown | react-markdown + remark-gfm |
| 状态 | `useReducer` (原生，无外部库) |
| HTTP | fetch 客户端 (按 domain 拆分 `lib/api/`) |
| SSE | `Response.body.getReader()` + `lib/sse/consumer.ts` |
| 构建 | Vite v5 |
| 类型 | TypeScript v5 |
| 测试 | Vitest v1 |
| 代码质量 | ESLint + Prettier |
| 桌面壳 | Tauri v2 (`@tauri-apps/api`) |

## State Management

`store/reducer.ts` — 中央状态 reducer：

```typescript
interface AppState {
  projects: Project[];
  activeProjectId: string | null;
  activeSessionId: string | null;
  phase: Phase;  // 'idle' | 'thinking' | 'callingTool' | 'streaming' | 'interrupted' | 'done'
}
```

**Project**:
```typescript
interface Project {
  id: string;           // 本质是 workingDir
  name: string;
  workingDir: string;
  sessions: Session[];
  isExpanded: boolean;
}
```

**Session**:
```typescript
interface Session {
  id: string;
  projectId: string;
  title: string;
  createdAt: number;
  updatedAt?: number;
  messages: Message[];
}
```

**Message 联合类型**:
```typescript
type Message =
  | UserMessage
  | AssistantMessage
  | ToolCallMessage
  | CompactMessage
  | SubRunStartMessage
  | SubRunFinishMessage;
```

**Reducer Actions** (17 种):

| Action | 职责 |
|--------|------|
| `INITIALIZE` | 初始化项目列表 |
| `SET_ACTIVE` | 切换活动项目/会话 |
| `SET_PHASE` | 更新运行阶段 |
| `ADD_PROJECT` | 添加项目并设为活动 |
| `ADD_SESSION` | 添加会话并设为活动 |
| `TOGGLE_EXPAND` | 项目展开/收起 |
| `DELETE_PROJECT` | 删除项目及自动切换 |
| `DELETE_SESSION` | 删除会话及自动切换 |
| `ADD_MESSAGE` | 追加消息到会话 |
| `UPSERT_USER_MESSAGE` | 按 turnId 更新/插入用户消息 |
| `APPEND_DELTA` | 追加 AI 文本增量 |
| `APPEND_REASONING_DELTA` | 追加推理增量 |
| `FINALIZE_ASSISTANT` | 完成 AI 消息 |
| `END_STREAMING` | 结束流式标记 |
| `APPEND_TOOL_CALL_DELTA` | 工具流式输出增量 |
| `UPDATE_TOOL_CALL` | 更新工具调用结果 |
| `REPLACE_SESSION_MESSAGES` | 替换整个会话消息 (回放) |

> 注意：`RENAME_PROJECT`、`RENAME_SESSION`、`SET_WORKING_DIR`、`ADD_SESSION_BACKEND` 在类型中声明但 reducer 中无对应 case 分支。

## Component Tree

```
<App>                          ← useReducer + 生命周期协调
├── <Sidebar>                  ← 侧边栏 (可拖拽 220-420px, localStorage 持久化)
│   ├── <ProjectItem> × N      ← 项目行 (可展开/折叠)
│   │   └── <SessionItem> × N  ← 会话行 (右键菜单)
│   └── <NewProjectModal>      ← 新建项目弹窗
├── <div role="separator">     ← 拖拽条
├── <Chat>                     ← 主聊天区
│   ├── <TopBar>
│   │   ├── <ModelSelector>    ← 模型选择
│   │   └── 侧边栏切换按钮
│   ├── <MessageList>          ← 消息列表 (含 ErrorBoundary)
│   │   ├── <UserMessage>
│   │   ├── <AssistantMessage> ← Markdown 渲染 + 思考块
│   │   ├── <ToolCallBlock>    ← 三种视图: diff / 终端 / 原始 JSON
│   │   └── <CompactMessage>   ← 压缩摘要
│   └── <InputBar>
│       ├── <CommandSelector>  ← / 命令选择器
│       └── <ModelSelector>
├── <SettingsModal>            ← 设置面板
└── <ConfirmDialog>            ← 全局确认对话框
```

## SSE 双通道

### 1. 会话事件流 (Session Event Stream)

```typescript
// useAgent.ts → connectSession()
GET /api/sessions/{sessionId}/events?afterEventId={cursor}
```

- **解析器**: `lib/sse/consumer.ts` — 按空行分隔 SSE 帧，提取 id/event/data 字段
- **事件归一化**: `lib/agentEvent.ts` (`normalizeAgentEvent()`) → `lib/applyAgentEvent.ts` → reducer dispatch
- **重连策略**: 指数退避 500ms → 1s → 2s → ...，上限 5s，最多 3 次尝试
- **防竞态**: `streamGenerationRef` 递增计数器，切换 session 时失效旧连接
- **断流检测**: `consumeSseStream` 返回 `ended` 触发重连

### 2. 目录事件流 (Session Catalog Event Stream)

```typescript
// useSessionCatalogEvents.ts (挂载后自动连接)
GET /api/session-events
```

- 监听: sessionCreated / sessionDeleted / projectDeleted / sessionBranched
- 重连策略: 指数退避，无最大次数，每次重连后调用 `onResync()` 刷新全量数据

## Auth Flow

`serverAuth.ts`:

1. **检测模式**: Tauri (`window.__ASTRCODE_BOOTSTRAP__`) / Dev (`GET /__astrcode__/run-info`) / URL query `?token=`
2. **交换 Token**: `POST /api/auth/exchange {token}` → 返回 session token (8h TTL)
3. **所有后续请求**: `x-astrcode-token` header
4. **SSE 认证**: `?token=` query param (EventSource 不支持自定义 header)
5. **自动续期**: `ensureServerSession()` 最多重试 2 次

## Key Files

| 文件 | 职责 |
|------|------|
| `src/App.tsx` | 主编排器，状态树，会话生命周期 |
| `src/store/reducer.ts` | 中央状态 reducer |
| `src/lib/agentEvent.ts` | SSE 事件协议规范化 (normalizeAgentEvent) |
| `src/lib/sse/consumer.ts` | SSE 流解析器 |
| `src/lib/applyAgentEvent.ts` | 事件 → reducer action 转换 |
| `src/hooks/useAgent.ts` | 主 API 协调 hook (CRUD + SSE 生命周期) |
| `src/hooks/useAgentEventHandler.ts` | SSE 事件 → reducer dispatch |
| `src/hooks/useSessionCatalogEvents.ts` | 全局目录事件监听 |
| `src/lib/api/` | API 客户端 (按 domain 拆分) |
| `src/lib/serverAuth.ts` | 多环境 Bootstrap 认证 |

## Tool Rendering

`ToolCallBlock` 三种渲染模式:

1. **Diff 视图** — `metadata.diff.patch` 存在 → 变更类型、路径、+/- 行数
2. **终端视图** — `metadata.display.kind === 'terminal'` → 命令、cwd、exit code、stdout/stderr
3. **原始输出** — 无 metadata → `<pre>` 纯文本；JSON 结构通过 `<ToolJsonView>` 展示
4. **子 Agent 分组** — 连续 `SubRunStartMessage`/`SubRunFinishMessage` 之间的消息渲染为 `agentGroup`

## Build Commands

```bash
cd frontend
npm run dev          # Vite 开发服务器 (port 5173)
npm run build        # 构建到 dist/
npm run typecheck    # 类型检查
npm run lint         # ESLint
npm run format:check # Prettier
```
