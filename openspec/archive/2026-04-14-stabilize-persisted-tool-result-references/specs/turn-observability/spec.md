## ADDED Requirements

### Requirement: turn observability SHALL 反映 persisted reference replacement 事实

turn 汇总与 observability 管线 MUST 反映 persisted reference replacement 的命中与重放情况，而不是只统计最终 prompt token 数。

#### Scenario: 发生 fresh replacement 时记录命中与节省量

- **WHEN** 本轮 request assembly 对 fresh tool result 应用了 persisted reference replacement
- **THEN** turn 汇总 SHALL 记录 replacement 命中数与节省的字节量
- **AND** 该指标 SHALL 能被 observability 管线累计

#### Scenario: 发生 replacement reapply 时记录稳定重放次数

- **WHEN** request assembly 重放之前已存在的 replacement 文本
- **THEN** turn 汇总 SHALL 记录 reapply 次数
- **AND** 该信息 SHALL 与 fresh replacement 命中区分开

#### Scenario: over-budget message 计数可被诊断读取

- **WHEN** 某轮 request assembly 命中了 aggregate tool-result budget
- **THEN** turn 汇总 SHALL 记录 over-budget message 数量
- **AND** 治理读取 SHALL 能区分“预算未命中”和“命中但无需新 replacement”的情况
