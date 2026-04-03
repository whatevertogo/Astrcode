# Frontend Architecture

React 18 + TypeScript SPA，无客户端路由，通过 SSE 双通道与 `crates/server` 实时通信.

## Tech Stack

| 层 | 技术 |
|---|------|
| UI | React 18 (Hooks), CSS Modules |
| 状态 | `useReducer` + `store/reducer.ts` |
| HTTP | 自定义 fetch 客户端 |
| SSE | `Response.body.getReader()` 流式读取 |
| 桌面壳 | Tauri (浏览器回退模式) |

## State Management

**`store/reducer.ts`** — 中央状态 reducer (~500 行):

```ts
interface AppState {
  projects: Project[];
  activeProjectId: string | null;
  activeSessionId: string | null;
  phase: Phase;  // idle | thinking | callingTool | streaming | interrupted | done
}
```

**Action 类别**:
- 会话生命周期: `INITIALIZE`, `SET_ACTIVE`, `ADD_SESSION`, `DELETE_SESSION`
- 消息流式: `APPEND_DELTA`, `APPEND_REASONING_DELTA`, `FINALIZE_ASSISTANT`, `END_STREAMING`
- 工具事件: `APPEND_TOOL_CALL_DELTA`, `UPDATE_TOOL_CALL`, `ADD_MESSAGE`
- 批量替换: `REPLACE_SESSION_MESSAGES` (快照加载)

## API Layer

统一 HTTP 客户端，自动注入 `x-astrcode-token`:

| 端点 | 方法 |
|------|------|
| `/api/sessions` | CRUD + prompt 提交 + 中断 |
| `/api/config` | 查询/保存 active selection |
| `/api/models/*` | 模型列表/当前模型/连接测试 |

## SSE Dual Channel

1. **Agent Events** (`/api/sessions/:id/events?afterEventId=<cursor>`): 单会话流，指数退底重连 (500ms-5000ms)
2. **Session Catalog** (`/api/session-events`): 全局会话创建/删除/分支事件

**关键设计模式**:
- `streamGenerationRef` / `sessionActivationGenerationRef` — 防止会话切换时陈旧 SSE 事件污染
- `turnSessionMapRef` — turnId → sessionId 映射，处理分支场景
- `startTransition()` — 流式更新不阻塞高优先级渲染

## Component Tree

```
App
  ├─ Sidebar (可拖拽 220-420px, localStorage 持久化)
  │    ├─ ProjectItem (可展开, 右键菜单)
  │    └─ SessionItem (右键菜单)
  └─ Chat
       ├─ TopBar → ModelSelector
       ├─ MessageList → UserMessage | AssistantMessage | ToolCallBlock
       └─ InputBar
```

## Tool Rendering

`ToolCallBlock` 根据 metadata 选择三种渲染模式:

1. **Diff 视图** — `metadata.diff.patch` 存在 → 变更类型、路径、+/- 行数
2. **终端视图** — `metadata.display.kind === 'terminal'` → 命令、cwd、exit code、stdout/stderr
3. **原始输出** — 无 metadata 驱动 → `<pre>` 纯文本

## Auth Flow

```
1. 检测模式: Tauri / Dev (port 5173) / 注入 __ASTRCODE_BOOTSTRAP__
2. POST /api/auth/exchange → bootstrap token 交换 session token
3. 所有后续请求通过 x-astrcode-token 头注入
4. ensureServerSession() — token 过期自动刷新
```

## Key Files

| 文件 | 职责 |
|------|------|
| `src/App.tsx` | 主编排器, 状态树, 会话生命周期 |
| `src/store/reducer.ts` | 中央状态 reducer |
| `src/lib/agentEvent.ts` | SSE 事件协议 v1 规范化器 |
| `src/lib/sse/consumer.ts` | SSE 流解析器 |
| `src/hooks/useAgent.ts` | 主 API 协调 hook (CRUD + SSE 生命周期) |
| `src/hooks/useAgentEventHandler.ts` | SSE 事件 → reducer dispatch |
