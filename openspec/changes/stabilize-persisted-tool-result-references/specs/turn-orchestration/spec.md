## ADDED Requirements

### Requirement: turn request assembly SHALL 应用 persisted tool-result references

`session-runtime` 的 turn request assembly MUST 在组装 LLM 请求前，对同一 API-level user tool-result 批次应用 persisted reference replacement，而不是把所有原始 tool result 原样塞进最终 prompt。

#### Scenario: over-budget tool-result batch 使用 persisted reference replacement

- **WHEN** 同一批 tool result 组成的 API-level user message 超过 aggregate tool-result budget
- **THEN** 系统 SHALL 将选中的 fresh tool result 替换为 `<persisted-output>` 引用文本
- **AND** 最终 LLM 请求 SHALL 使用 replacement 后的消息序列

#### Scenario: 已替换过的结果在后续轮次中重放相同 replacement

- **WHEN** 某个 `tool_call_id` 之前已经应用过 persisted reference replacement
- **THEN** 后续 request assembly SHALL 重放完全相同的 replacement 文本
- **AND** SHALL NOT 重新生成语义等价但字节不同的引用文本

#### Scenario: 已看过但未替换的结果不得在后续轮次补替换

- **WHEN** 某个 `tool_call_id` 之前已经进入 prompt 且未被替换
- **THEN** 后续 request assembly SHALL 保持其未替换状态
- **AND** SHALL NOT 在后续轮次中突然把它替换为 persisted reference
