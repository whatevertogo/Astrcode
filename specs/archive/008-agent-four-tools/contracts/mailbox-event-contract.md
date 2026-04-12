# Contract: Durable Mailbox 事件与回放语义

## 目标

定义四工具协作模型下 mailbox durable 真相源、事件顺序、回放规则和重复投递语义。

## Event: `AgentMailboxQueued`

### Purpose

记录一条刚刚成功进入 mailbox 的协作消息。

### Required Fields

| Field | Description |
|-------|-------------|
| `delivery_id` | 稳定唯一消息 ID |
| `from_agent_id` | 发送方 agent |
| `to_agent_id` | 接收方 agent |
| `message` | 协作正文 |
| `queued_at` | 入队时间 |
| `sender_lifecycle_status` | 入队时发送方生命周期快照 |
| `sender_last_turn_outcome` | 入队时发送方最近一轮结果快照 |
| `sender_open_session_id` | 入队时发送方会话目标快照 |

### Rules

- live inbox 只能在 `Queued` append 成功后更新
- 发送到 `Terminated` 目标时不得写入 `Queued`
- 快照字段全部是 enqueue-time snapshot

## Event: `AgentMailboxBatchStarted`

### Purpose

记录某个 agent 在本轮开始时通过 `snapshot drain` 接管了哪些消息。

### Required Fields

| Field | Description |
|-------|-------------|
| `agent_id` | 开始消费该批次的 agent |
| `turn_id` | 当前 durable turn |
| `batch_id` | 固定批次 ID |
| `delivery_ids` | 本轮接管的全部消息 ID |

### Rules

- 必须是 mailbox-wake turn 的第一条 durable 事件
- 仅表示“本轮接管了哪些消息”，不等于已确认处理完成
- turn 运行中新增消息不得并入该批次

## Event: `AgentMailboxBatchAcked`

### Purpose

记录一个 started batch 在 durable turn completion 后已被确认处理完成。

### Required Fields

| Field | Description |
|-------|-------------|
| `agent_id` | 处理该批次的 agent |
| `turn_id` | 对应 durable turn |
| `batch_id` | 批次 ID |
| `delivery_ids` | 已确认的消息 ID |

### Rules

- 必须在 durable turn completion 之后追加
- 不允许在模型流结束但 turn 尚未 durable 提交时提前 ack

## Event: `AgentMailboxDiscarded`

### Purpose

记录某个被 `close` 的 agent/subtree 主动丢弃的未 ack mailbox 消息。

### Required Fields

| Field | Description |
|-------|-------------|
| `agent_id` | 被丢弃 pending mailbox 的目标 agent |
| `delivery_ids` | 被丢弃的未 ack 消息 ID |

### Rules

- `close` 时必须先计算并 durable 记录所有未 ack pending delivery
- replay 时这些消息不得再重建为 pending

## Replay Rules

pending 集合按如下方式重建：

```text
pending = all Queued delivery_ids
        - all Acked delivery_ids
        - all Discarded delivery_ids
```

补充规则：

- `BatchStarted` 只标记 active batch，不从 durable 语义上移除 pending
- crash 发生在 `Started` 后、`Acked` 前时，相同 `delivery_id` 会再次进入 pending
- 该行为被定义为合法的 `at-least-once`

## Prompt Injection Rules

每条 mailbox 消息注入 prompt 时至少包含：

- `delivery_id`
- `from_agent_id`
- `sender_lifecycle_status`
- `sender_last_turn_outcome`
- `message`

系统提示必须明确告诉模型：

- 相同 `delivery_id` 可能因恢复而再次出现
- 相同 `delivery_id` 不应被当作全新任务重复处理

## Service-Side Guarantees

- 批内重复 `delivery_id` 在注入前必须先做服务端去重
- 不保证跨崩溃、跨 context window 的历史注入一定仍在当前窗口中可见
- 不能依赖动态 prompt 注入本身作为 durable transcript
