# Design: Child Session 协作运行时

## 目标

本设计回答四个问题：

1. 谁拥有 child session 的 durable 真相？
2. 协作 tool 与 runtime 投递层如何分层？
3. parent/child 如何双向通信且只消费一次？
4. 哪些现有边界需要删除或收口？

## 边界与 Owner

| Boundary | Owns | Must not own |
|---------|------|--------------|
| `core` | `ChildAgentRef`、协作 tool DTO、inbox/notification 领域类型、trait 契约 | runtime 默认值、tool 执行实现、server/frontend 启发式 |
| `runtime-agent-tool` | `spawn/send/wait/close/resume/deliver` 的工具适配、schema、结果投影 | child session orchestration、inbox 存储、session 持久化 |
| `runtime-execution` | child session orchestration、投递/唤醒/去重、parent reactivation、handoff/result 组装 | session durable ledger、tool registry 主入口 |
| `runtime-agent-control` | live agent handle、cancel token、运行态状态控制 | durable truth、父视图投影 |
| `runtime-session` / `storage` | session durable ledger、JSONL append/replay、child session 节点与通知事件写入 | live handle 控制、tool 业务决策 |
| `runtime-registry` | `CapabilityRouter`、tool→capability 适配、runtime context 构造 | child session 业务逻辑 |
| `server` | child session / parent summary 的 DTO 投影与 HTTP/SSE 路由 | durable 真相判断、UI 归并规则 |
| `frontend` | 父摘要视图、子会话完整视图、breadcrumb/read model | 反推 parent/child ownership 真相 |

## 协作工具契约层

模型侧看到的是一组显式的协作工具：

- `spawnAgent`
- `sendAgent`
- `waitAgent`
- `closeAgent`
- `resumeAgent`
- `deliverToParent`（只在 child session 中可见）

这些工具共同遵守三个约束：

1. 输入必须只描述目标 agent、意图和补充上下文，不描述 runtime 内部 transport 细节。
2. 输出必须是可消费的稳定结果语义，不能把内部事件流或 raw JSON 原样抛给模型。
3. 任一协作工具都只能触发“向某个目标 agent 投递一个 envelope”，不能直接修改其他 agent 内部状态。

## 运行时投递层

runtime 内部通过 `AgentInboxEnvelope` 实现协作语义：

1. 工具层接收模型输入并构造协作请求。
2. `runtime-execution` 把请求转换成 envelope，写入 durable event。
3. 如果目标 agent 当前在线，`runtime-agent-control` 负责唤醒或排队。
4. 目标 agent 消费 envelope 后，把结果写回自己的 session 和必要的 parent notification。
5. 若目标是父 agent，则 parent 在需要时被显式重新激活。

### 为什么不用递归 tool 执行

- 会把 child 内部 progress 暴露给 parent
- 恢复后难以保证单次消费
- tool 调用链递归后更难做取消与去重
- front/backend 都会被迫理解内部 transport 细节

## Durable 事件与会话关系

本 feature 的 durable 真相至少需要覆盖三类记录：

1. `ChildSessionNode`  
   记录 child session 节点、ownership、lineage 和执行边界。
2. `AgentInboxEnvelope`  
   记录谁向谁投递了什么协作输入，以及投递/消费状态。
3. `CollaborationNotification`  
   记录 child 如何向 parent 投影摘要、等待、完成、失败和关闭。

### 关键原则

- parent/child ownership 不能从磁盘路径推断
- child session transcript 必须留在 child session 自己的 durable history 里
- parent history 只保留 notification，不混入 child transcript
- legacy subrun 历史如果没有完整节点信息，必须显式降级

## Parent Reactivation

child 向 parent 交付结果时，不能只写一个 UI 通知；必须通过统一输入语义重新送达 parent：

1. 写入 parent durable notification
2. 生成 parent-targeted envelope
3. 若 parent 当前空闲但仍存活，唤醒 parent loop
4. 若 parent 已结束 turn，但 session 仍可继续服务用户，则开启新的 parent turn/step 来消费交付

这里的关键不是“马上回复用户”，而是“让 parent 真正知道自己该继续工作”。

## 与 Registry / Capability 的关系

本 feature 会进一步强化以下收口：

- `CapabilityRouter` 是生产执行唯一主入口
- `ToolRegistry` 仅作为测试/组装辅助
- `ToolCapabilityInvoker` 负责 tool → capability 投影
- runtime 默认 profile/context 的构造留在 `runtime-registry`，不回流到 `core`

原因是：协作工具族扩张后，任何 tool/capability 双轨都会立刻变成维护负担。

## Non-Goals

本轮不做：

- 独立消息队列或外部 broker
- 新数据库或全文索引
- 完整 fork prompt/caching 优化实现
- generalized multi-tenant agent orchestration

本轮只建立 clean architecture 下最小但完整的 child-session 协作底座。
