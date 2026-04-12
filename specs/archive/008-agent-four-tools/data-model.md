# Data Model: Astrcode Agent 协作四工具重构

## Core Domain Models

### AgentInstanceHandle

**Description**: 运行时内一个可被父 Agent 寻址、发送消息、观察和关闭的持久协作对象。实现上继续复用 `SubRunHandle` 名称，但语义升级为长期存在的 agent 句柄。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `agent_id` | `AgentId` | 协作对象的稳定 ID |
| `sub_run_id` | `SubRunId` | 该 agent 对应的子执行实例 ID |
| `session_id` | `SessionId` | durable 会话归属 |
| `open_session_id` | `OpenSessionId` | 供父级/前端打开的目标会话 |
| `parent_agent_id` | `Option<AgentId>` | 直接父 agent；root 为 `None` |
| `storage_mode` | `SubRunStorageMode` | 新写统一为 `IndependentSession` |
| `lifecycle_status` | `AgentLifecycleStatus` | `Pending/Running/Idle/Terminated` |
| `last_turn_outcome` | `Option<AgentTurnOutcome>` | 最近一轮执行结果 |
| `depth` | `u32` | 在控制树中的深度 |

### Validation Rules

- `Terminated` 后不得再次接收 `send`
- 非 root 节点必须拥有 `parent_agent_id`
- 新写节点的 `storage_mode` 必须为 `IndependentSession`

### State Transitions

| From | To | Trigger |
|------|----|---------|
| `Pending` | `Running` | 首轮 turn 开始 |
| `Running` | `Idle` | 单轮 durable completion 完成 |
| `Idle` | `Running` | 收到新 batch 并开始下一轮 |
| `Pending/Running/Idle` | `Terminated` | `close` 成功执行 |

### AgentMailboxEnvelope

**Description**: 父子 Agent 之间发送的一条 durable 协作消息，是 mailbox 的最小可恢复单元。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `delivery_id` | `DeliveryId` | 稳定唯一标识，用于恢复与去重 |
| `from_agent_id` | `AgentId` | 发送方 agent |
| `to_agent_id` | `AgentId` | 接收方 agent |
| `message` | `String` | 协作正文 |
| `queued_at` | `DateTime<Utc>` | 入队时间 |
| `sender_lifecycle_status` | `AgentLifecycleStatus` | 入队时发送方生命周期快照 |
| `sender_last_turn_outcome` | `Option<AgentTurnOutcome>` | 入队时发送方最近一轮结果快照 |
| `sender_open_session_id` | `OpenSessionId` | 入队时发送方可打开会话目标 |

### Validation Rules

- 快照字段一律是 **enqueue-time snapshot**，不是注入时现查
- `delivery_id` 必须在 durable 事件里保持稳定，重放时不得重写
- 发送到 `Terminated` 目标的 envelope 不得产生

## Runtime Coordination Structures

### AgentMailboxBatchRef

**Description**: 某个 agent 在单轮开始时通过 `snapshot drain` 接管的一批固定 mailbox 消息集合。它表达的是“本轮接管关系”，不是新的 durable 真相源。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `batch_id` | `BatchId` | 本轮固定批次 ID |
| `agent_id` | `AgentId` | 本轮消费该批次的目标 agent |
| `turn_id` | `TurnId` | 对应的 durable turn |
| `delivery_ids` | `Vec<DeliveryId>` | 本轮接管的全部消息 ID |
| `started_at` | `DateTime<Utc>` | 批次开始时间 |
| `acked_at` | `Option<DateTime<Utc>>` | 批次确认时间；未确认时为 `None` |

### Validation Rules

- 一轮 turn 只能有一个 active batch
- batch 中的消息集合在 `BatchStarted` 写入后不可变
- turn 运行中新到消息不得并入当前 batch

## Derived Read Models

### MailboxProjection

**Description**: 从 mailbox durable 事件重建出的派生读模型，用于 `observe`、wake 调度和恢复。它不是独立实体，也不是第二套真相源；唯一 durable 真相仍是 event log。

### Derived Fields

| Field | Type | Description |
|-------|------|-------------|
| `pending_delivery_ids` | `Vec<DeliveryId>` | `Queued - Acked - Discarded` 后剩余的待处理消息 |
| `active_batch_id` | `Option<BatchId>` | 当前 started-but-not-acked 的批次 |
| `active_delivery_ids` | `Vec<DeliveryId>` | 当前 active batch 中的消息 |
| `discarded_delivery_ids` | `Vec<DeliveryId>` | 因 `close` 而 durable 丢弃的消息 |
| `pending_message_count` | `usize` | 对外暴露的待处理消息数量 |

### Replay Rules

- `Queued` 增加 pending
- `BatchStarted` 标记 active batch，但不从 durable 语义上视为 ack
- `BatchAcked` 把对应消息移出 pending/active
- `Discarded` 把被关闭 agent 的未 ack 消息标记为丢弃并停止重建

### ObserveAgentResult

**Description**: `observe(agentId)` 返回的查询结果 DTO，融合 live control state、对话投影和 mailbox 派生信息。它是读模型，不是领域实体。

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `agent_id` | `AgentId` | 目标 agent |
| `sub_run_id` | `SubRunId` | 目标子执行实例 |
| `session_id` | `SessionId` | durable 会话 |
| `open_session_id` | `OpenSessionId` | 对应打开目标 |
| `parent_agent_id` | `AgentId` | 直接父 agent |
| `lifecycle_status` | `AgentLifecycleStatus` | 当前生命周期 |
| `last_turn_outcome` | `Option<AgentTurnOutcome>` | 最近一轮结果 |
| `phase` | `AgentPhase` | 来自现有对话投影 |
| `turn_count` | `u32` | 来自现有对话投影 |
| `active_task` | `Option<String>` | 当前正在处理的任务摘要；优先来自 active batch，否则回退到当前 turn 的任务上下文 |
| `pending_task` | `Option<String>` | 下一条待处理任务摘要；来自 pending mailbox 中尚未进入 active batch 的消息 |
| `pending_message_count` | `usize` | durable replay 为准的待处理数 |
| `last_output` | `Option<String>` | 最近 assistant 输出摘要 |

### Validation Rules

- 只有直接父级可以读取其直接子级的 `ObserveAgentResult`
- `pending_message_count` 对外值以 durable replay 为准，live cache 只做加速

## Durable Event Mapping

| Event | Durable Payload | Owner | Purpose |
|-------|-----------------|-------|---------|
| `AgentMailboxQueued` | `AgentMailboxEnvelope` | `core` 定义，`runtime` 追加 | 记录一条待处理协作消息 |
| `AgentMailboxBatchStarted` | `agent_id + turn_id + batch_id + delivery_ids` | `runtime` 追加 | 记录本轮接管的固定消息批次 |
| `AgentMailboxBatchAcked` | `agent_id + turn_id + batch_id + delivery_ids` | `runtime` 追加 | 记录某轮在 durable completion 后确认处理完成 |
| `AgentMailboxDiscarded` | `agent_id + delivery_ids` | `runtime` 追加 | 记录 close 时主动丢弃的 pending 消息 |

## Appendix: `observe` 字段来源矩阵

| Observe Field | Source of Truth |
|--------------|-----------------|
| `lifecycle_status` | live `AgentInstanceHandle` |
| `last_turn_outcome` | live `AgentInstanceHandle` |
| `phase` | `AgentStateProjector` |
| `turn_count` | `AgentStateProjector` |
| `active_task` | `MailboxProjection.active_delivery_ids` → 对应 `AgentMailboxEnvelope.message`；若当前无 active batch 且 lifecycle 为 `Pending/Running`，回退到当前 turn 最近一条用户任务消息 |
| `pending_task` | `MailboxProjection.pending_delivery_ids - active_delivery_ids` → 下一条 `AgentMailboxEnvelope.message` |
| `last_output` | `AgentStateProjector` |
| `pending_message_count` | `MailboxProjection` |
