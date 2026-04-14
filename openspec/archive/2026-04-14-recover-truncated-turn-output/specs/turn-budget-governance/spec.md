## ADDED Requirements

### Requirement: 输出截断恢复 SHALL 受显式尝试上限约束

`session-runtime` 对输出截断的 continuation 恢复 MUST 受显式上限约束，并使用正式配置控制，而不是无限继续或依赖调用方外部中断。

#### Scenario: 配置上限允许继续恢复

- **WHEN** 当前输出截断恢复次数低于 `max_output_continuation_attempts`
- **THEN** 系统可以继续注入下一条 continuation prompt

#### Scenario: 配置上限阻止继续恢复

- **WHEN** 当前输出截断恢复次数达到 `max_output_continuation_attempts`
- **THEN** 系统 SHALL 不再继续恢复
- **AND** turn 结束时 SHALL 带有明确的 stop cause

#### Scenario: 恢复次数与 budget 语义一致可观测

- **WHEN** turn 期间发生一次或多次输出截断恢复
- **THEN** 系统 SHALL 记录这些恢复次数
- **AND** 该信息 SHALL 能被 turn 级预算与汇总逻辑读取

