# measure-agent-tool-effectiveness 手工验收步骤

本文档用于验证 agent-tool 评估管线的三个核心诊断场景：

- 过度 `spawn`
- `observe` 轮询
- child reuse

所有步骤默认在本地开发环境执行，后端日志与调试读取面以同一套运行实例为准。

## 1. 过度 `spawn`

### 目标

确认主代理在同一轮内过度尝试创建 child 时：

- 会产生 `Spawn / Rejected` 协作事实
- facts 中包含稳定的策略上下文
- runtime observability snapshot 中的 `spawn_rejected` 与 rejection ratio 会增加

### 步骤

1. 将 `runtime.agent.maxSpawnPerTurn` 设为 `1`。
2. 启动应用并新建一个会话。
3. 让主代理在同一轮内尝试创建多个并行子代理，例如要求它“同时拆成多个子代理并行探索多个方面”。
4. 打开调试读取面或抓取 runtime metrics DTO。

### 期望

- parent session 的事件流中存在 `AgentCollaborationFact`：
  - `action = Spawn`
  - `outcome = Rejected`
  - `reason = spawn_budget_exhausted`
- fact 的 `policy_context.max_spawn_per_turn = 1`
- runtime metrics 中：
  - `agent_collaboration.spawn_accepted >= 1`
  - `agent_collaboration.spawn_rejected >= 1`
  - `agent_collaboration.spawn_rejection_ratio_bps` 不为空

## 2. `observe` 轮询

### 目标

确认重复 `observe` 不会只留下零散日志，而会体现在：

- `Observe / Accepted` 原始事实
- turn summary 中的 collaboration summary
- runtime observability 中的 `observe_calls` 与 `observe_to_action_ratio_bps`

### 步骤

1. 创建一个长时间运行的 child，让它执行较重的探索任务。
2. 在 child 尚未完成前，多次触发主代理观察该 child 状态。
3. 在其中一次 `observe` 后，显式让主代理采取下一步动作，例如 `send` 进一步细化任务或 `close` 结束该 child。
4. 获取当前 turn summary 与 runtime metrics DTO。

### 期望

- parent session 事件流中存在多条：
  - `action = Observe`
  - `outcome = Accepted`
- 至少一条后续事实属于：
  - `Send / Reused`
  - `Send / Queued`
  - `Close / Closed`
- 当前 turn summary 的 `collaboration.observe_calls > 0`
- runtime metrics 中：
  - `agent_collaboration.observe_calls > 0`
  - `agent_collaboration.observe_to_action_ratio_bps` 不为空

## 3. child reuse

### 目标

确认继续同一职责时系统优先复用已有 child，而不是无意义地再次 `spawn`，并能在评估面上体现 reuse。

### 步骤

1. 让主代理创建一个 child，用于探索某个明确模块。
2. 在 child 进入 `Idle` 后，再要求主代理继续推进同一模块的后续工作。
3. 观察主代理是否选择 `send` 给已有 child，而不是重新 `spawn` 新 child。
4. 获取 parent session 事件流与 runtime metrics DTO。

### 期望

- parent session 中出现：
  - `action = Send`
  - `outcome = Reused`
- 不应出现针对同一职责的额外 `Spawn / Accepted`
- runtime metrics 中：
  - `agent_collaboration.send_reused > 0`
  - `agent_collaboration.child_reuse_ratio_bps` 不为空

## 建议排查顺序

当指标与预期不一致时，按以下顺序排查：

1. 先看 parent session 是否已经写入 `AgentCollaborationFact`
2. 再看 turn summary 是否正确聚合同一 `turn_id` 的 facts
3. 最后看 runtime observability snapshot 是否正确累加全局状态

这样可以快速区分是记录点缺失、turn 聚合错误，还是全局 collector 聚合错误。
