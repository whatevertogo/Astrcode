# 前端架构

## 1. 目标

前端的职责不是定义协议，而是把 runtime 的 session / event 真相组织成可导航、可订阅、可操作的 UI。

当前前端要同时处理：

- 项目与会话列表
- 当前 session 的历史与增量事件
- subrun 聚焦视图
- child session 跳转
- 配置、模型和运行时状态

## 2. 基本形态

- React 18 + TypeScript SPA
- `useReducer` 作为中心状态管理
- `fetch` + SSE 作为主要数据通道
- Tauri 只作为宿主桥接，不定义前端领域模型

## 3. 状态边界

前端主状态至少包含：

- project / session catalog
- 当前活动 project / session
- 当前运行 phase
- `activeSubRunPath`
- 当前 session 的消息与事件派生视图

设计重点是把 **session** 与 **subrun** 视为不同对象，而不是都塞进同一个列表模型。

## 4. 数据来源

### 4.1 会话正文与执行增量

统一来自：

- `/api/sessions/{id}/history`
- `/api/sessions/{id}/events`

前端基于统一事件协议做 hydration 和增量更新。

### 4.2 全局目录变化

通过单独的 session catalog 事件流监听：

- session created / deleted
- project deleted
- branch / catalog refresh

### 4.3 子执行视图

subrun 视图来自同一 session 事件流的二次归并，而不是另一套独立消息协议。

## 5. 当前关键模块

| 模块 | 职责 |
| --- | --- |
| `src/store/reducer.ts` | 中心状态变更 |
| `src/hooks/useAgent.ts` | API + session SSE 协调 |
| `src/hooks/useSessionCatalogEvents.ts` | 目录事件监听 |
| `src/lib/agentEvent.ts` | 事件规范化 |
| `src/lib/applyAgentEvent.ts` | 事件到状态更新 |
| `src/lib/sessionView.ts` | session / subrun 视图参数与过滤 |
| `src/lib/subRunView.ts` | subrun 树与线程视图重建 |
| `src/lib/serverAuth.ts` | 服务端认证与 bootstrap |

## 6. SSE 策略

当前前端使用两类流：

1. **当前 session 事件流**：正文、工具、compact、subrun 生命周期
2. **全局目录事件流**：session / project 目录变化

约束是：

- 一条 session 一个连接
- 不为每个 subrun 单独开连接
- session 切换时必须使旧连接失效，避免竞态污染

## 7. 渲染模型原则

### 7.1 前端 render model 不是后端契约

后端只需要稳定提供事件；前端可以自由组织：

- message list
- tool card
- compact card
- subrun card
- focused subrun thread

### 7.2 subrun 聚焦基于路径而不是单点 ID

`activeSubRunPath` 比单个 `subRunId` 更适合表达嵌套子执行路径，也更利于 breadcrumb 和过滤视图。

### 7.3 child session 与 focused subrun 是两种导航动作

- focused subrun：仍停留在当前 session
- child session：跳到另一个 session

前端必须区分这两种跳转。

## 8. 前端当前不应承担的责任

- 定义后端事件语义
- 维护另一套 `/messages` 风格事实源
- 反向决定 session tree 的后端模型
- 把 subrun 生命周期识别写死成 `tool_name == "spawnAgent"`

## 9. 相关文档

- [../spec/session-and-subrun-spec.md](../spec/session-and-subrun-spec.md)
- [../design/multi-session-frontend-architecture.md](../design/multi-session-frontend-architecture.md)
