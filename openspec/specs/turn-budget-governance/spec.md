## Requirements

### Requirement: Token budget 驱动 turn 自动续写

`session-runtime` SHALL 在单次 turn 内根据 token budget 决策是否自动续写，而不是把继续/停止逻辑留给 `application`。

#### Scenario: 预算允许时注入 continue nudge

- **WHEN** 一轮 LLM 调用完成，且 budget 决策为继续
- **THEN** `session-runtime` 注入一条 auto-continue nudge 消息
- **AND** 继续下一轮 LLM 调用

#### Scenario: 达到停止条件时结束续写

- **WHEN** budget 决策为停止或收益递减
- **THEN** `session-runtime` 结束当前 turn
- **AND** 不再注入新的 continue nudge

---

### Requirement: 续写行为必须受硬上限约束

`session-runtime` SHALL 使用明确的 continuation 上限，防止单次 turn 无限续写。

#### Scenario: 达到最大续写次数

- **WHEN** continuation 次数达到配置上限
- **THEN** turn 停止自动续写
- **AND** 结束原因可被 observability 捕获

#### Scenario: 未达到上限且预算充足

- **WHEN** continuation 次数未达上限且 budget 允许继续
- **THEN** turn 可以继续执行下一轮
