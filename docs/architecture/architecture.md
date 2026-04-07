# Astrcode 项目架构总览

## 1. 文档定位

这份文档是 `docs/architecture/` 下的总入口，回答三个问题：

1. 整个项目按什么层次组织。
2. 后端 runtime / session / event / server 的主链路如何协作。
3. 前端如何作为后端事件协议的投影层工作。

这份文档重点在后端设计；前端只描述和后端契约直接相关的部分。更细的页面状态、组件拆分和交互策略，继续放在单独前端文档里。

---

## 2. 一句话架构

Astrcode 本质上是一个**以 durable event 为唯一事实源的 Agent Runtime**：

- Rust runtime 负责执行、会话、工具、子代理与持久化
- `server` 把 runtime 投影成 HTTP / SSE API
- `frontend` 基于 `/history + /events` 重建聊天与 subrun 视图

核心思想是：

- **后端定义真相**
- **传输层只做投影**
- **前端只做归并与渲染**

---

## 3. 总体分层

```text
protocol
   ↑
core
   ↑
storage / runtime-* / plugin
   ↑
runtime
   ↑
server
   ↑
src-tauri + frontend
```

### 3.1 `protocol`

只放跨边界 DTO，不放运行时策略、状态机和实现逻辑。

### 3.2 `core`

定义稳定核心契约：

- Tool / Capability / Policy
- Event 与 AgentContext
- Session / EventLog / Repository 接口
- Agent DTO、执行边界与共享类型

### 3.3 `storage` / `runtime-*` / `plugin`

这一层是能力实现层，但每个 crate 只负责一个明确职责：

- `storage`：append-only durable 持久化
- `runtime-session`：session / turn 生命周期与 durable 写入边界
- `runtime-agent-loop`：LLM → tool → LLM 主循环
- `runtime-execution`：执行装配
- `runtime-agent-control`：多 Agent 控制面
- `runtime-agent-tool`：把 `spawnAgent` 暴露成工具
- `runtime-prompt` / `runtime-llm` / `runtime-config`：独立子系统
- `plugin`：插件宿主

### 3.4 `runtime`

`runtime` 是门面层，负责把上面的能力装配成统一服务，不复制子 crate 逻辑。

### 3.5 `server`

负责把 runtime 投影成：

- HTTP 查询与 mutation
- SSE 增量事件流
- 认证与桌面壳可消费的 API 面

### 3.6 `src-tauri + frontend`

- `src-tauri`：桌面壳与本地桥接
- `frontend`：React SPA，只消费后端暴露的 session/event 协议

---

## 4. 后端设计主线

后端真正的主线不是“聊天 UI”，而是以下四个稳定契约。

### 4.1 Event Contract

这是系统最核心的契约。

- durable `StoredEvent` / `StorageEvent` 是唯一事实源
- replay cache、snapshot、前端 render model 都是派生层
- `/history` 和 `/events` 必须共享同一事件协议

这也是当前架构明确废弃 `/messages` 独立快照模型、统一到事件主线的原因。

### 4.2 Session Contract

`runtime-session` 负责：

- session 创建与重水合
- turn 执行生命周期
- durable 事件写入
- recent tail / compact / replay 相关状态

它**不负责**：

- 前端树形导航
- 子执行目录 read model
- UI 渲染友好的消息结构

换句话说，session 真相在后端，但 session tree 只是上层 read model。

### 4.3 Execution Contract

一次执行大致经过：

1. `server` 接收请求
2. `runtime` 选择 session 与执行入口
3. `runtime-execution` 装配 prompt、policy、tool surface、limits
4. `runtime-session` 建立 turn 边界并写 durable 事件
5. `runtime-agent-loop` 运行 LLM / tool 主循环
6. `storage` 落盘，`server` 同步广播给 SSE

这里最重要的分工是：

- `runtime-execution` 负责“怎么装”
- `runtime-agent-loop` 负责“怎么跑”
- `runtime-session` 负责“怎么记真相”

### 4.4 Transport Contract

`server` 不是第二事实源，只是 runtime 的投影层。

目前主要接口面：

- `GET /api/sessions/:id/history`
- `GET /api/sessions/:id/events`
- `GET /api/session-events`
- 其余 session / config / model / runtime mutation API

其中 `/history + /events` 是前端消费会话内容的主线。

---

## 5. Session、Turn 与 SubRun 的位置

这是理解整个项目的关键。

### 5.1 Session

`session` 是 durable 会话实体，代表一条可重放的历史。

### 5.2 Turn

`turn` 是一次执行边界，用来组织：

- 用户输入
- assistant 输出
- tool 调用
- turn 完成 / 中断 / compact

### 5.3 SubRun

`subrun` 不是新的顶层事实源，而是某个 turn 中通过 `spawnAgent` 触发的受控子执行。

后端通过以下事件表达 subrun 生命周期：

- `SubRunStarted`
- `SubRunFinished`

前端和父流程都应该基于这些生命周期事件识别和消费子执行，而不是基于工具名硬编码。

### 5.4 SharedSession vs IndependentSession

这两者只回答“事件写到哪里”：

- `SharedSession`：子执行事件写回父 session
- `IndependentSession`：子执行写入独立 child session

它们不应该改变任务 ownership、控制责任或运行时事实源。

---

## 6. 前端在整体架构中的位置

前端不是 session 真相所在层，而是**事件协议的归并层**。

它的主要职责是：

1. 通过 `/history` 做首屏 hydration
2. 通过 `/events` 接收增量更新
3. 把统一事件协议归并成 render model
4. 提供根 session、subrun 视图、breadcrumb 和 child session 跳转

当前前端已经从“后端 `/messages` 快照”切到“统一事件协议重建 UI”的主线，这和后端架构方向一致。

### 6.1 当前前端复杂度来自哪里

当前复杂度主要集中在会话视图编排层，而不是整体系统分层本身。原因是前端同时要处理：

- 根 session
- 同 session 内的 subrun 过滤视图
- 独立 child session 跳转
- SSE 连接与重连
- URL 状态与 breadcrumb

这会让 `App.tsx` 一类编排入口显得偏重，但它不代表后端主线混乱。

---

## 7. 必须守住的架构边界

### 7.1 必须遵守

- `protocol` 不依赖 `core` / `runtime`
- `runtime-*` 尽量通过 `core` 契约协作
- `storage` 实现持久化接口，而不是把存储细节反推回 `core`
- `runtime` 只做聚合与装配
- `server` 只做传输投影，不定义新的领域真相

### 7.2 当前刻意不做

- 不把前端 render model 下沉成后端核心模型
- 不把 session tree 做成 runtime 基础领域对象
- 不让子 Agent 控制面和 session 落盘模式耦合
- 不为了 UI 方便而重新引入第二套快照协议

---

## 8. 推荐阅读顺序

1. [./architecture.md](./architecture.md)
2. [./crates-dependency-graph.md](./crates-dependency-graph.md)
3. [../design/runtime-session-and-turn-lifecycle.md](../design/runtime-session-and-turn-lifecycle.md)
4. [../design/agent-tool-and-api-design.md](../design/agent-tool-and-api-design.md)
5. [./frontend-architecture.md](./frontend-architecture.md)

---

## 9. 相关文档

- [./README.md](./README.md)
- [./crates-dependency-graph.md](./crates-dependency-graph.md)
- [./frontend-architecture.md](./frontend-architecture.md)
- [./skills-architecture.md](./skills-architecture.md)
- [../design/runtime-session-and-turn-lifecycle.md](../design/runtime-session-and-turn-lifecycle.md)
- [../design/agent-tool-and-api-design.md](../design/agent-tool-and-api-design.md)
- [../spec/README.md](../spec/README.md)
