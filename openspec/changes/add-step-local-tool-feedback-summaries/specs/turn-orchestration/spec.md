## ADDED Requirements

### Requirement: tool cycle 之后 SHALL 生成可供下一轮消费的反馈包

`session-runtime` 在一个 step 的 tool cycle 完成后 MUST 能生成 step-local 的工具反馈包，用于下一轮 request assembly 消费，而不是只能把原始 tool result 直接堆回消息历史。

#### Scenario: 读密集工具回合生成反馈包

- **WHEN** 一个 step 内完成一批只读或等价可清理工具调用
- **THEN** 系统 SHALL 生成对应的工具反馈包
- **AND** 下一轮 request assembly SHALL 可以消费该反馈包

#### Scenario: 原始工具事实仍然保留

- **WHEN** 系统为某个 step 生成了工具反馈包
- **THEN** 原始 tool result 事实 SHALL 仍然保留用于 durable append、replay 与调试
- **AND** 反馈包 MUST NOT 替代原始事件日志

#### Scenario: 反馈包覆盖范围可识别

- **WHEN** 一个反馈包覆盖了若干 `tool_call_id`
- **THEN** request assembly SHALL 能识别这些覆盖范围
- **AND** 能据此减少重复把同一批原始结果再次塞入 prompt

