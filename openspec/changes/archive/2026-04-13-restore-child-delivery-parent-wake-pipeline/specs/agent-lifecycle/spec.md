## ADDED Requirements

### Requirement: Parent delivery batch lifecycle

kernel 与 application SHALL 为 parent delivery batch 定义稳定生命周期，使 child 终态回流具备可重试与可观测行为。

#### Scenario: Delivery batch enters waking state

- **WHEN** 系统 checkout 一批父级交付用于 wake
- **THEN** 该批次进入“正在唤醒父级”的中间状态
- **AND** 在被 consume 或 requeue 前不得被重复消费

#### Scenario: Busy parent defers batch consumption

- **WHEN** 父级当前忙碌，无法立即开始 wake turn
- **THEN** 该批次保持或恢复为待重试状态
- **AND** MUST NOT 被提前 consume

#### Scenario: Successful wake consumes batch

- **WHEN** 父级 wake turn 成功接受并完成该批次
- **THEN** 系统从 parent delivery queue 中消费该批次

#### Scenario: Failed wake keeps batch retryable

- **WHEN** 父级 wake turn 提交失败或中途失败
- **THEN** 系统重新排队该批次
- **AND** SHALL 记录对应失败信号供观测使用
