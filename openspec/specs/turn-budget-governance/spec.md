## Purpose

规范化 turn 内续写行为的预算治理规则，约束 `session-runtime` 在可观测、可追踪路径上的续写决策。
## Requirements
### Requirement: Token budget 驱动 turn 自动续写

`session-runtime` SHALL 在单次 turn 内根据 token budget 决策是否自动续写，而不是把继续/停止逻辑留给 `application`；当调用方显式提供 token budget 时，系统 SHALL 以显式输入作为本次 turn 的正式 budget 来源。

#### Scenario: 预算允许时注入 continue nudge

- **WHEN** 一轮 LLM 调用完成，且 budget 决策为继续
- **THEN** `session-runtime` 注入一条 auto-continue nudge 消息
- **AND** 继续下一轮 LLM 调用

#### Scenario: 达到停止条件时结束续写

- **WHEN** budget 决策为停止或收益递减
- **THEN** `session-runtime` 结束当前 turn
- **AND** 不再注入新的 continue nudge

#### Scenario: Explicit token budget overrides default for one turn

- **WHEN** 调用方为本次执行显式提供 token budget
- **THEN** `session-runtime` SHALL 使用该值作为本次 turn 的 budget
- **AND** 不修改全局默认配置

### Requirement: 续写行为必须受硬上限约束

`session-runtime` SHALL 使用明确的 continuation 上限，防止单次 turn 无限续写。

#### Scenario: 达到最大续写次数

- **WHEN** continuation 次数达到配置上限
- **THEN** turn 停止自动续写
- **AND** 结束原因可被 observability 捕获

#### Scenario: 未达到上限且预算充足

- **WHEN** continuation 次数未达上限且 budget 允许继续
- **THEN** turn 可以继续执行下一轮

### Requirement: budget 决策 SHALL 产出稳定的 continuation 与 stop cause

`session-runtime` 在 turn 内做 budget 决策时 MUST 产出稳定的 continuation cause 或 stop cause，而不是只返回一个无法解释的布尔结果。该原因 SHALL 能被 loop、测试和 observability 重用。

#### Scenario: budget 允许继续时产生 continuation cause

- **WHEN** 本轮 assistant 输出完成且 budget 决策允许继续
- **THEN** 系统生成稳定的 continuation cause
- **AND** 该 cause SHALL 被 turn loop 用于注入 continue nudge 并进入下一轮

#### Scenario: budget 阻止继续时产生 stop cause

- **WHEN** 本轮 assistant 输出完成且 budget 决策要求停止
- **THEN** 系统生成稳定的 stop cause
- **AND** turn loop SHALL 使用该原因结束当前 turn

#### Scenario: hard limit 停止同样必须有原因

- **WHEN** continuation 次数达到上限或等价硬限制
- **THEN** 系统生成稳定的 stop cause
- **AND** 该 stop cause SHALL 不依赖调用方额外推断

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
