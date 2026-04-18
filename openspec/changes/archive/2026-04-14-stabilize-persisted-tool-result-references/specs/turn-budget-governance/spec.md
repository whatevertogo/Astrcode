## ADDED Requirements

### Requirement: turn budget governance SHALL 覆盖 aggregate tool-result budget

`session-runtime` MUST 对单个 API-level user tool-result 批次应用 aggregate tool-result budget，而不是只依赖每个工具自己的 inline limit。

#### Scenario: fresh tool results largest-first 地被替换

- **WHEN** 一批 fresh tool results 的总内容超过 aggregate tool-result budget
- **THEN** 系统 SHALL 从最大的 fresh 结果开始应用 persisted reference replacement
- **AND** 持续替换直到该批次降到 budget 内或 fresh 候选耗尽

#### Scenario: 未超预算的批次保持原样

- **WHEN** 一批 tool results 的总内容未超过 aggregate tool-result budget
- **THEN** 系统 SHALL 保持该批次原样
- **AND** SHALL NOT 为了统一格式而额外替换为 persisted reference

#### Scenario: 已 compacted 或不参与 replacement 的结果被跳过

- **WHEN** 某个 tool result 已经是 `<persisted-output>` 引用或不属于可参与 aggregate replacement 的内容类型
- **THEN** 系统 SHALL 跳过该结果
- **AND** SHALL NOT 对其再次应用 aggregate replacement
