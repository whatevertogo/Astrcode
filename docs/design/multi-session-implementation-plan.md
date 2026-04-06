# 多会话前端架构实施计划

## 1. 目标

在不改写当前 session 真相的前提下，逐步实现：

- 父会话内展示 subrun 生命周期
- `SharedSession` 的同 session 子执行树 / 过滤视图
- `IndependentSession` 的 child session 跳转
- 后续可扩展的 subrun read model 与过滤能力

> 设计前提：session 与 turn 生命周期以  
> [runtime-session-and-turn-lifecycle](./runtime-session-and-turn-lifecycle.md) 为准。  
> 多会话前端结构以 [multi-session-frontend-architecture](./multi-session-frontend-architecture.md) 为准。

---

## 2. 当前基线

### 已有能力

- `GET /api/sessions`
- `GET /api/sessions/{id}/history`
- `GET /api/sessions/{id}/events`
- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`
- `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`
- `SubRunStarted / SubRunFinished` 生命周期事件
- `SubRunFinished.result` 结构化 handoff

### 当前限制

- 还没有 session 级 `list_subruns` read model
- 前端还在从事件流本地构造 subrun 树
- 服务端还没有 `subRunId + scope` 过滤能力
- `IndependentSession` 仍属 experimental
- `/messages` 仍存在兼容链路，但不应再作为主线继续依赖

---

## 3. 成功标准

完成后至少应满足：

1. 父会话里能稳定看到 subrun 卡片与状态变化
2. 用户能打开 `SharedSession` 子执行树视图，而无需新建伪 session
3. 当 `childSessionId` 存在时，用户能跳到独立 child session
4. 同一 `sessionId` 只保留一条 SSE 连接
5. 后续若新增 `list_subruns` / server-side filter，也不需要推翻已有前端状态模型

---

## 4. 分阶段实施

## 阶段 0：文档定锚

### 目标

先把 session 真相、统一事件协议和前端导航边界固定下来，避免后续实现建立在过时文档模型上。

### 交付

- `runtime-session-and-turn-lifecycle.md`
- 收口后的内容架构文档
- 收口后的多会话前端设计与实施计划
- 明确 `/messages` 进入废弃路径，主线统一到 `/history + /events`

### 验收

- 不再在多份文档里重复定义 `Session / SubSession / SessionRepository`
- 明确 `subrun != child session`
- 明确 `StorageEvent -> AgentEventEnvelope -> frontend render model` 主线
- 明确 `SessionMeta.parent_session_id` 不是通用 subrun tree 字段

---

## 阶段 1：前端侧 MVP（不新增 API）

### 目标

直接依赖现有 session history/events，先把当前会话内的 subrun 导航做起来。

### 任务

- 引入 `ActiveView = session | subRun`
- 在前端从 `history/events` 中提取 `SubRunRef`
- 在根会话中渲染真正的嵌套子执行树
- 新增 `SubRunConversationView`
- 实现 `SharedSession` 的同 session 子树过滤视图
- 如果 `childSessionId` 存在，支持跳到独立 session

### 关键实现点

1. 打开会话时：
   - 加载 `/history`
   - 用统一事件协议做首屏 hydration
2. 运行中：
   - 订阅 `/events`
   - 将新的 `SubRunStarted / SubRunFinished` 归并到本地索引
3. 树构建规则：
   - 用 `sub_run_id` 建立节点
   - 用 `parent_turn_id` + turn owner 推导父子 subrun 关系
   - 根视图展示完整树；subrun 视图展示当前子树

### 验收

- 在已有 session 页面中可点击 subrun 卡片进入子执行视图
- 根视图中 subrun 以树形方式展示，而不是扁平混排
- SharedSession 不新增额外 SSE 连接
- IndependentSession 在 `childSessionId` 存在时可跳转

---

## 阶段 2：补 durable subrun read model

### 目标

为前端提供更轻量、更可复用的 subrun 列表接口，减少每次都扫全量 history。

### 推荐新增 API

- `GET /api/v1/sessions/{id}/subruns`

### 设计要求

这个 read model 必须：

1. **先基于 durable 事件重建**
   - 以 `SubRunStarted / SubRunFinished` 为主
2. **再按需用 live `AgentControl` 补 running 状态**
3. **不能只依赖控制面内存注册表**
   - 因为 finalized handle 会被裁剪
   - durable history 才是完整来源

### 验收

- 前端可仅用 `list_subruns` 构建当前 session 的 subrun 摘要列表
- running/completed 状态与单个 `subruns/{id}` 查询保持一致
- 返回结果足以支持子执行树与 breadcrumb 构建

---

## 阶段 3：IndependentSession 体验补强

### 目标

让独立 child session 的导航与返回路径更自然。

### 任务

- 在 breadcrumb 中保留 `Session → SubRun → Child Session` 路径
- 从父卡片跳转 child session 时携带足够的返回信息
- child session 页面保留“返回父 subrun / 父会话”入口

### 验收

- child session 的跳入/返回路径清晰可预测
- 不需要把 `child_sessions[]` 直接塞入 `SessionMeta`

---

## 阶段 4：服务端过滤与协议补强

### 目标

在 MVP 和 durable subrun read model 稳定后，再做性能与协议补强。

### 推荐新增能力

- `GET /api/sessions/{id}/history?subRunId=...&scope=self|subtree|directChildren`
- `GET /api/sessions/{id}/events?subRunId=...&scope=self|subtree|directChildren`

### 设计要求

- 不建议只做 `subRunId` equality filter
- 过滤语义必须能支撑嵌套子执行树
- `sub_run_id` 应是主公开过滤键；`agent_id` 仅作调试/内部补充

### 验收

- 当前 subrun 视图可通过服务端直接拉取其子树事件
- 不会因为过滤过窄而丢失下一层 subrun 生命周期

---

## 阶段 5：协议关联规则补强

### 目标

让 `spawnAgent` tool call 与 `SubRunStarted / SubRunFinished` 的关联不再依赖前端猜测。

### 推荐方案

优先补强生命周期事件与 tool call 的稳定关联：

1. **首选**：在 subrun 生命周期事件中补 `tool_call_id`
2. **兼容**：若暂时不补字段，至少把“同 turn 内按顺序 1:1 配对”写成显式协议约束

### 验收

- 同一 turn 内多个 `spawnAgent` 时，前端仍能稳定把 tool card 升级为正确的 subrun card

---

## 5. 实施顺序建议

按投入产出比排序，建议顺序是：

1. 文档定锚
2. 前端侧 MVP（本地提取 subrun 树）
3. durable `list_subruns` API
4. IndependentSession 导航补强
5. server-side `subRunId + scope` filter
6. subrun ↔ toolCall 关联规则补强

这条顺序的核心原因是：

- 当前已经有足够的事件面可做 MVP
- `subrun` read model 比 `session tree` 更贴近当前真实执行对象
- 过早做 tree 容易把错误模型固化进 API
- 简单 equality filter 不值得单独做一版，最好直接一步到位定义 `scope`

---

## 6. 当前阶段不建议做的事

- 扩展 `SessionMeta.child_sessions`
- 把 `/api/v1/sessions/tree` 设为第一阶段 blocker
- 只靠 `AgentControl.list()` 做 subrun 列表
- 为 SharedSession 的每个 subrun 开单独 SSE
- 把 `IndependentSession` 当成当前默认主线
- 继续把 `/messages` 当成必须维护的主线快照协议

---

## 7. 里程碑检查表

## M1：文档与状态模型稳定

- [x] 新增 runtime-session 设计文档
- [x] 收口内容架构文档
- [x] 收口多会话前端设计文档
- [x] 收口本实施计划
- [x] 明确 `/messages` 进入废弃路径

## M2：前端 MVP

- [x] 建立 `ActiveView(session | subRun)` 基础导航模型
- [x] 从 `history/events` 构建 `SubRunRef`
- [x] 渲染 `SubRunCard`
- [x] 支持 SharedSession 过滤视图
- [x] 支持 child session 跳转
- [x] 根视图改为真正的嵌套子执行树

## M3：durable subrun read model

- [ ] 新增 `GET /api/v1/sessions/{id}/subruns`
- [ ] 用 durable events 重建状态
- [ ] 与单个 `subruns/{id}` 查询对齐

## M4：增强与优化

- [ ] breadcrumb 完整化
- [ ] 独立 child session 返回路径
- [ ] 追加 `subRunId + scope` server-side filter
- [ ] 明确 subrun ↔ toolCall 关联规则

---

## 8. 当前阶段结论

多会话前端现在最合适的推进方式不是“先造一棵全局 session tree”，而是：

1. 先承认 **session 是 durable 真相单位**
2. 再把 **subrun 作为导航 read model** 做出来
3. 主线统一到 **`/history + /events`**
4. 然后才考虑更高层的 filter / tree / cross-session 聚合
