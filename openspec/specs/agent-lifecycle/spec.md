## Requirements

### Requirement: Agent 邮箱消息收发

kernel/agent_tree SHALL 为每个注册的 agent 维护一个 inbox，支持异步消息推送和批量消费。

#### Scenario: 推送消息到 agent inbox

- **WHEN** 调用 `push_inbox(agent_id, message)`
- **THEN** 消息被追加到该 agent 的 inbox 队列，如果 agent 正在等待则唤醒

#### Scenario: 消费 agent inbox

- **WHEN** 调用 `drain_inbox(agent_id)`
- **THEN** 返回所有待处理消息并清空 inbox

#### Scenario: 阻塞等待 inbox 消息

- **WHEN** 调用 `wait_for_inbox(agent_id)` 且 inbox 为空
- **THEN** 当前 task 挂起直到有新消息到达

---

### Requirement: Agent 状态观察

kernel/agent_tree SHALL 支持观察子代理的实时状态。

#### Scenario: 查询子代理状态

- **WHEN** 调用 `observe(agent_id)`
- **THEN** 返回该 agent 当前的 lifecycle stage、正在执行的 turn 信息、inbox 大小

#### Scenario: 观察不存在的 agent

- **WHEN** 调用 `observe` 传入未注册的 agent_id
- **THEN** 返回错误 `AgentControlError::NotFound`

---

### Requirement: Agent 路由

kernel/agent_tree SHALL 支持按条件路由消息到目标 agent。

#### Scenario: 路由到指定 agent

- **WHEN** 调用 `route(agent_id, message)`
- **THEN** 消息被投递到目标 agent 的 inbox

#### Scenario: 路由到不存在 agent

- **WHEN** 目标 agent_id 未注册
- **THEN** 返回路由失败错误

---

### Requirement: Agent 唤醒机制

kernel/agent_tree SHALL 支持唤醒处于 idle 状态的 agent 执行新的 turn。

#### Scenario: 唤醒 idle agent

- **WHEN** 调用 `wake(agent_id, trigger)` 且 agent 处于 idle 状态
- **THEN** agent 被唤醒执行新的 turn

#### Scenario: 唤醒 busy agent

- **WHEN** 调用 `wake` 但 agent 当前正在执行 turn
- **THEN** 触发被缓存，agent 完成当前 turn 后自动检查并执行

---

### Requirement: 取消传播

kernel/agent_tree SHALL 支持递归取消 agent 及其所有子 agent。

#### Scenario: 取消 agent 树

- **WHEN** 调用 `cancel_tree(agent_id)`
- **THEN** 该 agent 及所有子 agent 被标记为 cancelled，所有正在执行的任务收到取消信号

#### Scenario: 按 parent turn 取消

- **WHEN** 调用 `cancel_for_parent_turn(turn_id)`
- **THEN** 所有属于该 parent turn 的子 agent 被取消
