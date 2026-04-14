## ADDED Requirements

### Requirement: turn loop SHALL 记录显式 transition 原因

`session-runtime` 的 turn loop MUST 为每一次“继续下一轮”的动作记录显式 transition 原因，而不是仅依赖分散的局部计数器或隐式分支。该 transition 原因 SHALL 由 `session-runtime/turn` 持有，并驱动后续 request 重新组装、LLM 重试或 budget 续写。

#### Scenario: tool 结果驱动下一轮

- **WHEN** 一轮 LLM 输出包含工具调用，且 tool cycle 成功完成
- **THEN** turn loop 记录一次显式 transition，表示“工具结果已追加，进入下一轮”
- **AND** 下一轮 prompt 重新组装 SHALL 以该 transition 为当前 continue 原因

#### Scenario: reactive compact 驱动下一轮

- **WHEN** 一轮 LLM 调用因 prompt-too-long 被恢复为 reactive compact 成功路径
- **THEN** turn loop 记录一次显式 transition，表示“压缩恢复后重新尝试”
- **AND** 系统 SHALL 在不落入普通完成路径的前提下重新组装请求

#### Scenario: budget 允许 auto-continue

- **WHEN** turn loop 在一次 assistant 输出后判断 budget 允许继续
- **THEN** turn loop 记录一次显式 transition，表示“budget 允许续写”
- **AND** 系统注入对应的 continue nudge 后进入下一轮

