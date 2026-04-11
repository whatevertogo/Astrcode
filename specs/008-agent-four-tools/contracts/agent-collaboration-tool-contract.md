# Contract: Agent 协作公开工具

## 目标

定义 `spawn`、`send`、`observe`、`close` 四个公开协作工具的稳定合同，作为 `runtime-agent-tool`、`core` DTO、prompt 描述、server API 与前端调用层的一致依据。

## Tool: `spawn`

### Purpose

创建一个新的持久 child agent，并返回后续协作使用的稳定 `agentId`。

### Request Fields

| Field | Required | Description |
|-------|----------|-------------|
| `task` | yes | 子 agent 的首轮工作描述 |
| `type` | no | 子 agent 类型或 profile 提示 |
| `context` | no | 供首轮工作使用的附加上下文 |

### Response Fields

| Field | Description |
|-------|-------------|
| `agentId` | 新 child agent 的稳定标识 |
| `subRunId` | 对应子执行 ID |
| `openSessionId` | 可打开会话目标 |

### Behavioral Rules

- 新创建 child 一律使用 `IndependentSession`
- 新 child 初始生命周期为 `Pending`
- 首轮开始后生命周期切为 `Running`

## Tool: `send`

### Purpose

向直接父或直接子发送一条 durable 协作消息。`send` 是唯一公开消息通道。

### Request Fields

| Field | Required | Description |
|-------|----------|-------------|
| `agentId` | yes | 目标 agent |
| `message` | yes | 协作消息正文 |

### Response

- 成功时返回空结果或最小确认
- 错误时必须显式返回非法路由、目标已终止或上下文缺失等原因

### Permission Rules

- 仅允许直接父 -> 直接子
- 仅允许直接子 -> 直接父
- 禁止兄弟、越级、跨树、伪造目标

### Delivery Rules

- 目标为 `Idle` 时：消息入队后触发目标下一轮
- 目标为 `Running` 时：消息只入队，不插入当前轮
- 目标为 `Terminated` 时：直接报错，不入 mailbox，不唤醒

## Tool: `observe`

### Purpose

返回目标 child 的增强快照，供父 agent 决策是否继续 `send` 或执行 `close`。

### Request Fields

| Field | Required | Description |
|-------|----------|-------------|
| `agentId` | yes | 被观测的 child agent |

### Response Fields

| Field | Description |
|-------|-------------|
| `agentId` | 目标 agent |
| `subRunId` | 目标子执行 ID |
| `sessionId` | durable 会话 |
| `openSessionId` | 可打开会话目标 |
| `parentAgentId` | 直接父 agent |
| `lifecycleStatus` | 当前生命周期状态 |
| `lastTurnOutcome` | 最近一轮执行结果 |
| `phase` | 当前对话阶段 |
| `turnCount` | 当前轮次数 |
| `pendingMessageCount` | durable replay 为准的待处理消息数量 |
| `lastOutput` | 最近输出摘要 |

### Permission Rules

- 只有直接父可以观测直接子
- 非直接父、兄弟、跨树调用必须拒绝

## Tool: `close`

### Purpose

终止目标 child agent 及其后代，是唯一公开终止手段。

### Request Fields

| Field | Required | Description |
|-------|----------|-------------|
| `agentId` | yes | 目标 child agent |

### Behavioral Rules

- 关闭目标及其整个子树
- 若运行中，先取消当前 turn
- durable 丢弃未 acked 的 mailbox 消息
- 清理 pending wake item
- 生命周期进入 `Terminated`

### Non-Goals

- 不支持只关闭单节点保留后代
- 不支持公开 `resume`

## Removed Public Surface

以下名称必须从公开 schema、prompt、工具注册表和调用层中彻底消失：

- `waitAgent`
- `sendAgent`
- `closeAgent`
- `deliverToParent`
- `resumeAgent`

## Error Contract

| Code | Meaning |
|------|---------|
| `invalid_route` | 非直接父子关系或跨树路由 |
| `agent_terminated` | 目标 agent 已终止，拒收 `send` |
| `observe_forbidden` | 非直接父调用 `observe` |
| `missing_agent_context` | 当前 turn 缺少调用所需的 agent 上下文 |
| `subtree_close_failed` | subtree 终止过程中出现内部错误 |
