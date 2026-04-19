## Requirements

### Requirement: 活跃任务注册表

application lifecycle SHALL 提供 `TaskRegistry`，跟踪 turn 和 subagent 的 JoinHandle，注册时自动清理已完成 handle，shutdown 时批量 abort。

#### Scenario: 注册 turn 任务

- **WHEN** 调用 `register_turn_task(handle)`
- **THEN** 清理已完成的旧 turn handle，将新 handle 追加到 turn_handles

#### Scenario: 注册 subagent 任务

- **WHEN** 调用 `register_subagent_task(handle)`
- **THEN** 清理已完成的旧 subagent handle，将新 handle 追加到 subagent_handles

#### Scenario: shutdown 批量 abort

- **WHEN** 调用 `take_all_turn_handles()` / `take_all_subagent_handles()`
- **THEN** 返回所有 handle 并清空内部 Vec，供调用方统一 abort

---

### Requirement: Agent 邮箱消息收发

kernel AgentControl SHALL 为每个注册的 agent 维护一个 inbox，支持异步消息推送和批量消费。

#### Scenario: 推送消息到 agent inbox

- **WHEN** 调用 `push_inbox(sub_run_or_agent_id, envelope)`
- **THEN** 消息被追加到该 agent 的 inbox 队列，如果 agent 正在等待则唤醒

#### Scenario: 消费 agent inbox

- **WHEN** 调用 `drain_inbox(sub_run_or_agent_id)`
- **THEN** 返回所有待处理消息并清空 inbox

#### Scenario: 阻塞等待 inbox 消息

- **WHEN** 调用 `wait_for_inbox(sub_run_or_agent_id)` 且 inbox 为空
- **THEN** 当前 task 挂起直到有新消息到达

---

### Requirement: Agent 状态观察

kernel KernelAgentSurface SHALL 支持查询代理的实时状态。

#### Scenario: 查询子代理状态

- **WHEN** 调用 `query_subrun_status(agent_id)`
- **THEN** 返回 `SubRunStatusView`（含 agent_id、profile、lifecycle_status、depth、turn_outcome、resolved_limits、delegation、storage_mode、open child session id）

#### Scenario: 查询根代理状态

- **WHEN** 调用 `query_root_status(session_id)`
- **THEN** 返回该 session 根代理的 `SubRunStatusView`

#### Scenario: 获取 handle

- **WHEN** 调用 `get_handle(sub_run_or_agent_id)`
- **THEN** 返回 `Option<SubRunHandle>`

#### Scenario: 获取 lifecycle 和 turn outcome

- **WHEN** 调用 `get_lifecycle(sub_run_or_agent_id)` → `Option<AgentLifecycleStatus>`
- **WHEN** 调用 `get_turn_outcome(sub_run_or_agent_id)` → `Option<AgentTurnOutcome>`

---

### Requirement: Agent 控制树生命周期

kernel AgentControl SHALL 管理代理在控制树中的注册、状态转换和取消传播。

#### Scenario: 注册根代理

- **WHEN** 调用 `register_root_agent(agent_id, session_id, profile_id)`
- **THEN** 创建 depth=0 的 SubRunHandle 并插入控制树，返回 handle

#### Scenario: 注册独立子代理

- **WHEN** 调用 `spawn_independent_child(profile, parent_session_id, child_session_id, parent_turn_id, parent_agent_id)`
- **THEN** 校验深度和并发限制，创建子 SubRunHandle（depth = parent depth + 1），插入控制树

#### Scenario: 设置 lifecycle

- **WHEN** 调用 `set_lifecycle(id, new_status)`
- **THEN** 更新 agent 的 lifecycle status（如 Running → Idle），如果 agent 正在等待则唤醒

#### Scenario: 完成 turn

- **WHEN** 调用 `complete_turn(id, outcome)`
- **THEN** 原子更新 lifecycle 和 turn outcome，释放并发槽位

#### Scenario: resume agent

- **WHEN** 调用 `resume(sub_run_or_agent_id)`
- **THEN** 将 agent 设置为 Running，唤醒等待者

---

### Requirement: 取消传播

kernel AgentControl SHALL 支持递归取消 agent 及其所有子 agent。

#### Scenario: 取消单个 agent

- **WHEN** 调用 `cancel(sub_run_or_agent_id)`
- **THEN** 该 agent 被标记为 cancelled，所有正在执行的任务收到取消信号

#### Scenario: 按 parent turn 取消

- **WHEN** 调用 `cancel_for_parent_turn(parent_turn_id)`
- **THEN** 所有属于该 parent turn 的子 agent 被取消，返回受影响的 handle 列表

#### Scenario: 关闭子树

- **WHEN** 调用 `close_subtree(agent_id)`
- **THEN** 递归关闭 agent 及其所有后代，返回 `CloseSubtreeResult`（含 closed_count 和 closed_agent_ids）

#### Scenario: 终止子树

- **WHEN** 调用 `terminate_subtree(sub_run_or_agent_id)`
- **THEN** 递归终止 agent 及其所有后代，从控制树中移除

---

### Requirement: 父级 delivery 队列

kernel AgentControl SHALL 管理父级 delivery 缓冲队列，支持 enqueue / checkout / requeue / consume 操作。

#### Scenario: 入队 delivery

- **WHEN** 调用 `enqueue_parent_delivery(parent_session_id, parent_turn_id, notification)`
- **THEN** delivery 被排入指定 session 的 parent delivery buffer

#### Scenario: 批量取出

- **WHEN** 调用 `checkout_parent_delivery_batch(parent_session_id)`
- **THEN** 返回该 session 的所有待处理 delivery（批量），并从 buffer 中移除

#### Scenario: 重新入队

- **WHEN** 调用 `requeue_parent_delivery_batch(parent_session_id, delivery_ids)`
- **THEN** 指定 delivery 被重新放回 buffer（用于 wake turn 失败后重试）

#### Scenario: 消费确认

- **WHEN** 调用 `consume_parent_delivery_batch(parent_session_id, delivery_ids)`
- **THEN** 从 durable 存储中移除已消费的 delivery

---

### Requirement: Agent 等待

kernel AgentControl SHALL 支持等待 agent turn 完成。

#### Scenario: 等待 agent idle

- **WHEN** 调用 `wait(sub_run_or_agent_id)`
- **THEN** 当前 task 挂起直到该 agent 的 lifecycle 变为 Idle，返回最新 handle

---

### Requirement: 应用层治理

application lifecycle SHALL 提供 `AppGovernance`，管理运行时的生命周期、可观测性和重载能力。

#### Scenario: 治理快照

- **WHEN** 调用 `snapshot(plugin_search_paths)`
- **THEN** 返回 `GovernanceSnapshot`（含 runtime_name、runtime_kind、session count、running sessions、metrics、capabilities、plugins）

#### Scenario: 优雅关闭

- **WHEN** 调用 `shutdown(timeout_secs)`
- **THEN** 先 abort 所有 turn 和 subagent 任务，再关闭运行时和托管组件

#### Scenario: 重载

- **WHEN** 调用 `reload()` 且无运行中 session
- **THEN** 执行运行时重载，返回 `ReloadResult`（含新 snapshot 和 reloaded_at 时间戳）
- **WHEN** 有运行中 session
- **THEN** 返回 `ApplicationError::Conflict`
