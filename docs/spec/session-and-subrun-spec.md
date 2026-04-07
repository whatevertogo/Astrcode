# Session / Turn / SubRun 规范

## 1. 范围

本文档统一定义：

- runtime-session 的职责边界
- Session / Turn / SubRun / Child Session 的关系
- durable 事件与传输事件的内容语义
- 多会话前端导航与 SSE 约束

它替代原先分散在设计稿、计划稿和内容架构稿里的重复定义。

## 2. 术语

| 术语 | 含义 |
| --- | --- |
| Session | durable 会话实体 |
| Turn | 一次根执行回合 |
| SubRun | 一次子执行实例 |
| Child Session | 仅在 `IndependentSession` 下创建的独立 session |
| StoredEvent | durable 事实源 |
| AgentEventEnvelope | `/history` 与 `/events` 统一传输信封 |

## 3. 核心真相

### 3.1 durable 事件是唯一事实源

下列能力都必须建立在 durable 事件之上：

- 首屏 hydration
- replay
- compact tail
- subrun 列表重建
- child session 导航

`recent_records`、render model、前端缓存都不是事实源。

### 3.2 runtime-session 的职责边界

它负责：

- session 创建与重水合
- turn 生命周期管理
- durable 事件追加
- token budget 与 recent tail 状态
- compaction 相关状态维护

它不负责：

- 前端 session tree
- 高层 child navigation read model
- UI 卡片结构
- “一页看所有子会话”的产品化组织

### 3.3 session tree 是 read model

如果需要会话树或 child navigation：

- 可以建立在 durable 事件之上
- 但不能把 session tree 反向固化为 runtime-session 核心领域模型

## 4. Turn 生命周期

主链路应保持以下分段：

1. `prepare_session_execution`
2. `run_session_turn`
3. `execute_turn_chain`
4. `complete_session_execution`

这样可以保证：

- abort 有稳定边界
- replay 有稳定边界
- compaction 有稳定边界
- observability 有稳定边界

## 5. Session / SubRun / Child Session

### 5.1 `SharedSession`

- 子执行事件写入父 session
- 父会话可以直接消费 `SubRunStarted / SubRunFinished`
- 是当前正式主线

### 5.2 `IndependentSession`

- 子执行事件写入独立 child session
- 父侧通过 `child_session_id` 与 `SubRunFinished` 建立跳转关系
- 当前仍为 experimental

### 5.3 领域规则

- `SubRun` 不是 `Session`
- `Child Session` 不是“另一个普通 subrun 卡片”
- task ownership 与 session ownership 必须分离

## 6. 内容模型

### 6.1 三层内容模型

1. durable 事件层：`StoredEvent`
2. 传输事件层：`AgentEventEnvelope`
3. 前端归并层：message / tool / subrun / compact / error 等 render model

### 6.2 `/history` 与 `/events`

二者必须返回同一套 `AgentEventEnvelope` 语义：

- `/history`：首屏 hydration / 回放
- `/events`：SSE 增量

`/messages` 不再作为主线兼容接口维护。

### 6.3 关键事件语义

| 事件 | 语义 |
| --- | --- |
| `UserMessage` | 用户正文 |
| `AssistantDelta` / `AssistantFinal` | 助手正文 |
| `ThinkingDelta` | thinking 内容 |
| `ToolCall` / `ToolCallDelta` / `ToolResult` | 工具调用与结果 |
| `CompactApplied` | 上下文压缩边界 |
| `SubRunStarted` / `SubRunFinished` | 子执行生命周期 |
| `Error` | 错误事件 |
| `TurnDone` | 执行边界，不必渲染为正文消息 |

### 6.4 `CompactApplied`

`CompactApplied` 的核心语义是“上下文边界发生变化”，不是“普通聊天消息”。

后端只需稳定提供事件；前端可以把它渲染为：

- 分隔线
- 摘要卡片
- 折叠提示

### 6.5 `spawnAgent` 的内容语义

- `spawnAgent` 本身仍然是普通 tool call
- 子执行后续进展不通过 tool result 持续更新
- 前端识别 subrun 的核心依据是生命周期事件，而不是工具名

因此协议必须为 tool call 与 subrun 生命周期提供稳定关联。

## 7. 前端导航模型

### 7.1 导航对象

前端至少区分三种视图：

1. 根 session 视图
2. SharedSession 的 subrun 视图
3. IndependentSession 的 child session 视图

### 7.2 状态模型建议

前端可以自由设计具体 TypeScript 结构，但应至少能够表达：

- session catalog
- `subRunsBySession`
- 当前 `subRunPath`
- `childSessionId`
- 当前视图范围（root / subtree / direct children 等）

### 7.3 SSE 约束

当前推荐：

- 一条 session 一个连接
- 不为每个 subrun 单独开连接
- 当前阶段优先客户端归并和过滤

## 8. 扩展能力

以下能力是可选增强，不是当前主线的前置条件：

### 8.1 durable subrun read model

可新增：

- `GET /api/v1/sessions/{id}/subruns`

但它必须：

- 基于 durable 事件重建
- 再按需用 live 控制面补 running 状态
- 不能只看内存注册表

### 8.2 server-side filter

如果 history / events 载荷持续增大，可考虑增加：

- `subRunId`
- `scope(self|subtree|directChildren)`

作为服务端过滤能力。

## 9. 非目标

- 不把前端 render model 固化成后端协议
- 不把 session tree 作为 runtime 核心对象
- 不为当前阶段恢复 `/messages` 兼容接口

## 10. 对应文档

- 设计入口：[../design/runtime-session-and-turn-lifecycle.md](../design/runtime-session-and-turn-lifecycle.md)
- 前端设计入口：[../design/multi-session-frontend-architecture.md](../design/multi-session-frontend-architecture.md)
- 开放项：[./open-items.md](./open-items.md)
