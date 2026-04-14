## ADDED Requirements

### Requirement: 工具反馈打包命中情况 SHALL 可观测

`session-runtime` 对 step-local 工具反馈打包的命中情况 MUST 可观测，至少覆盖是否生成反馈包、覆盖了多少工具调用，以及是否减少了进入下一轮 prompt 的原始结果负担。

#### Scenario: 反馈包生成被记录

- **WHEN** 某个 step 成功生成工具反馈包
- **THEN** 系统 SHALL 记录该命中事实

#### Scenario: 覆盖范围被记录

- **WHEN** 某个反馈包覆盖了若干 `tool_call_id`
- **THEN** 系统 SHALL 记录覆盖规模或等价诊断信息

#### Scenario: 未命中原因可诊断

- **WHEN** 某个 step 没有生成反馈包
- **THEN** 系统 SHALL 能提供未命中的原因或等价诊断信息
- **AND** 治理读取方 SHALL 不必重扫原始消息才能判断是否命中

