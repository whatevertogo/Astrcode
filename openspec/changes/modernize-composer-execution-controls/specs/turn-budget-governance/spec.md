## MODIFIED Requirements

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
