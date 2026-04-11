# Design: 协作运行时边界与消息流

## 设计目标

- 对外只暴露 `spawn/send/observe/close`
- root 与 child 使用同一棵控制树
- child 单轮完成后回到 `Idle`
- mailbox durable 真相源统一落在 session event log
- prompt 注入继续是动态的，但不再承担 durable 真相角色

## 非目标

- 本轮不公开 `resume`
- 本轮不支持只关闭单节点保留后代
- 本轮不试图获得 exactly-once mailbox 语义
- 本轮不重写现有 `AgentStateProjector` 的对话职责

## 边界与 Owner

| Boundary | Single Owner | Responsibility |
|----------|--------------|----------------|
| `crates/core` | 协作契约 owner | 四工具 DTO、lifecycle/outcome 枚举、mailbox durable 事件、snapshot 协议 |
| `crates/runtime-agent-control` | live control owner | root/child 树关系、`SubRunHandle`、live inbox cache、wake queue、ancestor 校验 |
| `crates/runtime` | 执行编排 owner | `send/observe/close` 路由、root 注册、snapshot drain、mailbox 事件追加顺序 |
| `crates/runtime-session` + `crates/storage` | durable append owner | session event log 追加与恢复 |
| `crates/runtime-agent-loop` + `crates/runtime-prompt` | prompt owner | mailbox batch 动态注入、few-shot、工具描述 |
| `crates/server` + `frontend` | 调用面 owner | API/Hook/前端命名迁移与 surface 收敛 |

## 运行时主流程

### 1. `spawn`

1. 父 agent 或 root 调用 `spawn`
2. runtime 解析父执行上下文
3. 若父为 root，则先保证 root 已被注册进 `agent_control`
4. 创建新的 child handle，storage mode 固定为 `IndependentSession`
5. child 生命周期置为 `Pending`
6. 首轮开始后切为 `Running`

### 2. 父 -> 子 `send`

1. 校验调用方是否为直接父
2. 校验目标 child 不为 `Terminated`
3. 生成稳定 `delivery_id`
4. 先 append `AgentMailboxQueued`
5. append 成功后更新 live inbox/cache
6. 若 child 为 `Idle`，登记下一轮启动；若为 `Running`，只排队

### 3. 子 -> 父 `send`

1. 校验调用方是否为直接子
2. 生成 `delivery_id`
3. append `AgentMailboxQueued`
4. append 成功后更新父级 live inbox/cache
5. 父空闲则尝试立即启动下一轮；忙碌则保留 wake item

### 4. turn 开始与 batch 接管

1. 某个 agent 即将开始新一轮
2. 对当前 pending mailbox 做一次 `snapshot drain`
3. 生成固定 `batch_id`
4. 先写 `AgentMailboxBatchStarted`
5. 再构造本轮 mailbox prompt 注入
6. 轮中新增消息不并入本轮

### 5. turn 完成与 ack

1. 当前轮完成 durable turn result 提交
2. 更新该 agent 的 `last_turn_outcome`
3. 若未被 `close`，把 `lifecycle_status` 置回 `Idle`
4. 追加 `AgentMailboxBatchAcked`

### 6. `observe`

1. 校验调用方是否为直接父
2. 从 live handle 读取 lifecycle/outcome
3. 从 `AgentStateProjector` 读取 `phase` / `turn_count` / `last_output`
4. 从 mailbox projector 读取 pending/active batch 派生摘要
5. 组装 `AgentSnapshot`

### 7. `close`

1. 计算目标 subtree
2. 取消运行中的 turn
3. durable 追加对应 agent/subtree 的 `AgentMailboxDiscarded`
4. 清理 pending wake item
5. 生命周期全部置为 `Terminated`
6. 后续 `send` 到这些 agent 一律报错

## Durable 与 Live 的职责分工

### Durable 真相源

- session event log 中的 mailbox 事件
- 会话内已有的对话/turn 结果事件

### Live Overlay

- `runtime-agent-control` 的 inbox cache
- `runtime-agent-control` 的 wake queue
- 当前运行中 turn 的瞬时状态

### 设计原则

- live 状态只能在 durable append 成功后更新
- replay 结果必须不少于 live 对外可见值
- live overlay 加速读取，不负责定义历史真相

## 错误与可观测性

必须保留显式错误或结构化日志的场景：

- 非直接父子 `send`
- `send` 到 `Terminated`
- `observe` 非法权限
- root agent 注册失败
- mailbox append 失败
- `BatchStarted` 后 prompt 注入失败
- `close` 过程中 subtree 清理失败

日志建议最少包含：

- `agent_id`
- `parent_agent_id`
- `delivery_id`
- `batch_id`
- `turn_id`
- `lifecycle_status`
- `pending_message_count`

## 为什么不采用新 router / 新 WAL

- 当前仓库已经有 session event log 和 runtime control/live overlay；再引入新 WAL 会制造第二套 durable 真相源
- 新 router 不能替代 `agent_control` 的父子树与权限关系，最终仍要和现有结构对接
- 复用现有边界更符合仓库“一个边界一个 owner”的原则
