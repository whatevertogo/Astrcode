## ADDED Requirements

### Requirement: workflow phase bridge SHALL 交接执行上下文而不改写 task durable truth

当 workflow 从 planning 类 phase 迁移到 executing 类 phase 时，系统 MUST 通过显式 bridge 交接执行上下文，但 SHALL NOT 因该 bridge 自动创建、覆盖或清空 execution task 的 durable snapshot。`taskWrite` 仍 MUST 是 execution task truth 的唯一写入口。

#### Scenario: approved plan 进入 executing phase 时只注入 bridge context

- **WHEN** 一个 approved canonical plan 触发 workflow 从 `planning` phase 迁移到 `executing` phase
- **THEN** 系统 SHALL 向 executing phase 提供可消费的 bridge context
- **AND** SHALL NOT 在没有显式 `taskWrite` 调用的情况下生成新的 active task snapshot

#### Scenario: replan 回路不隐式清空现有 task snapshot

- **WHEN** executing phase 因用户触发 `replan` 类信号而回到 planning phase
- **THEN** 系统 SHALL NOT 自动清空现有 execution task durable snapshot
- **AND** task 面板的变化仍 SHALL 只由后续显式 task snapshot 写入驱动
