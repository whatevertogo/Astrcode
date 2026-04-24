## ADDED Requirements

### Requirement: turn orchestration SHALL 归属 `agent-runtime`

turn loop、provider 调用、tool dispatch、hook dispatch、stop/continue 判定与取消传播 MUST 由新 `agent-runtime` 统一拥有。workflow、discovery、plugin origin 判定等宿主逻辑 SHALL 不再直接嵌入 turn loop。

#### Scenario: turn loop 只围绕执行主链工作
- **WHEN** 一次 turn 从 prompt 开始执行
- **THEN** `agent-runtime` SHALL 只负责 `prompt -> provider -> tool/hook dispatch -> continue/stop`
- **AND** SHALL NOT 在 loop 中自行做 plugin discovery、theme/prompt/skill 搜索或 workflow 特判装配

### Requirement: turn orchestration SHALL 消费统一 hooks 事件点

turn orchestration MUST 在执行主链中显式触发统一 hooks 事件点，而不是只支持窄版 tool/compact hooks。

#### Scenario: turn 执行过程中触发扩展后的事件点
- **WHEN** 一次 turn 完整执行
- **THEN** 系统 SHALL 至少触发 `context`、`before_agent_start`、`before_provider_request`、`tool_call`、`tool_result`、`turn_start`、`turn_end`
- **AND** 这些事件 SHALL 由统一 hooks 平台接管
