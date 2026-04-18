## ADDED Requirements

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

