# 多会话前端架构设计

## 1. 目标

前端需要在不破坏当前 session 真相的前提下，支持下面三类体验：

1. 在父会话中看到子执行（subrun）的启动、运行中与完成状态
2. 在 `SharedSession` 模式下查看“同一 session 内的子执行视图”
3. 在 `IndependentSession` 模式下跳转到独立 child session

> 文档边界：session/turn 生命周期真相见  
> [runtime-session-and-turn-lifecycle](./runtime-session-and-turn-lifecycle.md)。  
> 本文只定义前端如何消费现有 session 与 subrun 投影。

---

## 2. 当前现实与约束

## 2.1 当前稳定后端面

当前已经存在并可直接利用的能力：

- `GET /api/sessions`
- `GET /api/sessions/{id}/messages`
- `GET /api/sessions/{id}/history`
- `GET /api/sessions/{id}/events`
- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`
- `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`

## 2.2 当前稳定语义

- `spawnAgent + controlled sub-session` 是当前主线
- `SharedSession` 是正式路径
- `IndependentSession` 仍属 experimental
- `SubRunStarted / SubRunFinished` 是父侧稳定生命周期事件
- `SubRunFinished.result` 是父流程和 UI 的结果中心

## 2.3 当前不要做错的事

下面这些假设目前都不应该当成前端的基础前提：

- 把 `SessionMeta` 直接扩展成完整 session tree
- 把 `SessionMeta.parent_session_id` 当成通用 subrun tree 字段
- 假设每个 subrun 都有 `child_session_id`
- 假设必须先做 `/api/v1/sessions/tree` 才能做多会话 UI

---

## 3. 设计原则

### 3.1 session 与 subrun 是两种不同导航对象

- `session`：真正的 durable 对话归属单位
- `subrun`：同一 session 内或 child session 上的子执行视图入口

所以前端导航至少需要两种 target：

- session view
- subrun view

### 3.2 先做“当前会话内可导航”，再做“全局会话树”

从现有接口出发，最自然的第一步是：

- 当用户已经打开某个 session 时
- 前端直接从该 session 的 `history/events` 里提取 `SubRunStarted / SubRunFinished`
- 基于此构建本地 `SubRunRef` 列表与导航入口

这一步不要求新的全局 tree API。

### 3.3 `SharedSession` 优先做客户端过滤视图

在 `SharedSession` 下：

- 子执行内容仍属于父 session
- 因此前端最先需要的是 **同 session 的过滤视图**
- 而不是再为它人为制造一个“伪 session”

---

## 4. 建议的前端状态模型

```ts
type ActiveView =
  | { kind: 'session'; sessionId: string }
  | { kind: 'subRun'; sessionId: string; subRunId: string };

interface SubRunRef {
  subRunId: string;
  sessionId: string;
  parentTurnId?: string;
  agentProfile?: string;
  storageMode?: 'sharedSession' | 'independentSession';
  childSessionId?: string;
  status: 'running' | 'completed' | 'failed' | 'aborted' | 'tokenExceeded';
  summary?: string;
}

interface SessionsState {
  catalog: Map<string, SessionListItem>;
  subRunsBySession: Map<string, Map<string, SubRunRef>>;
  activeView: ActiveView;
  connections: Map<string, SessionConnection>;
}
```

### 4.1 为什么 `subRunsBySession` 要独立存在

因为：

- `subrun` 不是 `SessionMeta` 子节点
- 它主要来自 `SubRunStarted / SubRunFinished` 事件归并
- 它既可能指向“同 session 过滤视图”，也可能指向“独立 child session”

---

## 5. 数据来源与构建方式

## 5.1 Session catalog

来源：

- `GET /api/sessions`

用途：

- 侧边栏会话列表
- 根 session 切换
- 显示标题、时间、phase 等摘要

## 5.2 当前会话的 subrun 列表

### MVP：直接从历史事件构建

来源：

- `GET /api/sessions/{id}/history`
- `GET /api/sessions/{id}/events`

构建规则：

1. 扫描 `SubRunStarted`
2. 以 `sub_run_id` 建立 `SubRunRef`
3. 收到 `SubRunFinished` 后更新状态、summary、childSessionId 等字段

这个方案的优点：

- 不依赖新 API
- durable
- 与现有 session truth 保持一致

### 后续优化：新增 `GET /api/v1/sessions/{id}/subruns`

如果后面要优化首屏载荷或避免全量 history 扫描，可以补一个只读 API：

- `GET /api/v1/sessions/{id}/subruns`

但它应当：

- 先基于 durable 事件重建 subrun 列表
- 再按需用 live `AgentControl` 丰富 running 状态

而不是只看控制面内存注册表。

## 5.3 单个 subrun 状态

来源：

- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`

用途：

- 某张 subrun 卡片打开后，做一次精确状态刷新
- running/completed 边缘切换时做补偿查询

---

## 6. 导航模型

## 6.1 三种视图

### A. 根 session 视图

展示：

- 当前 session 的完整对话
- 其中夹杂 subrun 卡片

### B. SharedSession 的 subrun 视图

本质上是：

- 仍然停留在同一个 `sessionId`
- 但只展示与某个 `sub_run_id` 相关的内容

过滤维度通常包括：

- `agent.sub_run_id == 当前 sub_run_id` 的事件
- 该 subrun 的 `SubRunStarted / SubRunFinished`
- 必要时显示父会话中的上下文入口卡片

### C. IndependentSession 的 child session 视图

本质上是：

- 真正切换到 `child_session_id`
- 仍保留从父 subrun 派生而来的 breadcrumb

---

## 7. SSE 连接策略

## 7.1 一条 session 一个连接

建议规则：

- 对同一个 `sessionId` 只建立一条 SSE 连接
- 不为每个 subrun 再开一条额外连接

这样做的原因：

- `SharedSession` 的子执行本来就在同一个 session stream 中
- 多条同 session SSE 连接会放大浏览器和服务端压力

## 7.2 subrun 过滤优先在客户端完成

MVP 先使用客户端过滤：

```ts
function belongsToSubRun(event: AgentEvent, subRunId: string): boolean {
  return event.data.agent?.subRunId === subRunId;
}
```

这样可以更快落地，也不需要马上扩展后端。

## 7.3 服务端过滤是优化项，不是前置条件

如果后续发现：

- `history/events` 载荷太大
- 页面切换时本地过滤成本明显上升

再考虑追加：

- `/api/sessions/{id}/events?subRunId=...`
- 或 history 的 server-side filter

但这应该是优化项，而不是第一阶段 blocker。

---

## 8. 推荐 UI 组件划分

| 组件 | 作用 |
|---|---|
| `SessionSidebar` | 展示 session catalog |
| `SessionConversation` | 渲染当前 session 视图 |
| `SubRunCard` | 展示子执行的 running/completed 状态与摘要 |
| `SubRunConversationView` | 渲染 SharedSession 下的 subrun 过滤视图 |
| `SessionBreadcrumb` | 展示 `Session → SubRun → Child Session` 导航路径 |
| `SessionEventManager` | 管理 SSE 连接、断点续传与分发 |

### 8.1 `SubRunCard` 最少要展示什么

- agent profile
- 当前状态
- `storageMode`
- `summary`（若已完成）
- `childSessionId` 对应的打开入口（若存在）
- 取消按钮（仅 running 且有权限时）

---

## 9. 推荐交互流

## 9.1 从父会话打开 SharedSession 子执行

```text
用户在父会话中点击 SubRunCard
    ↓
activeView = { kind: 'subRun', sessionId: 父会话, subRunId }
    ↓
复用同一个 session SSE 连接
    ↓
前端按 subRunId 过滤历史与增量事件
    ↓
展示 SubRunConversationView
```

## 9.2 从父会话跳到 IndependentSession child session

```text
用户在父会话中点击“打开独立会话”
    ↓
读取 card 上的 childSessionId
    ↓
activeView = { kind: 'session', sessionId: childSessionId }
    ↓
加载 child session messages/history
    ↓
breadcrumb 保留父 subrun 入口
```

---

## 10. 分阶段落地建议

### 阶段 A：不新增 API 的 MVP

- 使用 `/sessions/{id}/messages` + `/history` + `/events`
- 本地提取 `SubRunRef`
- 实现 `SubRunCard`
- 实现 SharedSession 子执行过滤视图
- 如果 `childSessionId` 存在，支持跳转独立 session

### 阶段 B：补 durable subrun read model

- 新增 `GET /api/v1/sessions/{id}/subruns`
- 用于更轻量地构建 subrun 列表和摘要
- 与单个 `subruns/{sub_run_id}` 状态查询对齐

### 阶段 C：做更高层的 tree / filter 优化

- server-side `sub_run_id` filter
- session tree 聚合接口
- 跨会话 breadcrumbs 优化

---

## 11. 当前阶段明确不建议的方向

- 一开始就重做 `SessionMeta` 结构
- 先做全局 `/sessions/tree` 再做子执行导航
- 为 SharedSession 的每个 subrun 单独创建 SSE 连接
- 把 subrun 当成新的 session 真相层级

---

## 12. 当前阶段结论

前端多会话设计现在最稳的主线是：

1. **session 仍然是 durable 真正单位**
2. **subrun 是前端导航对象，不是新的底层 session 模型**
3. **SharedSession 先用同 session 过滤视图实现**
4. **IndependentSession 再通过 `childSessionId` 做跳转增强**
