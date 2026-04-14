## ADDED Requirements

### Requirement: turn loop SHALL 在流式 LLM 阶段组装可执行工具候选并提前调度安全工具

`session-runtime` 的 turn loop MUST 能在接收 LLM `ToolCallDelta` 的过程中先组装可执行工具候选，并对“参数已闭合、调用 identity 稳定且能力声明为 concurrency-safe”的工具调用提前开始执行，而不是始终等待完整 `LlmOutput` 返回。

#### Scenario: 只读工具在流式阶段提前执行

- **WHEN** LLM 流式产出的一组 `ToolCallDelta` 已组装成一个参数已闭合、identity 稳定且 `concurrency_safe` 的工具调用候选
- **THEN** 系统 SHALL 在 assistant 完整输出结束前就开始该工具执行
- **AND** 该执行 SHALL 仍然属于当前 step

#### Scenario: 副作用工具保持保守调度

- **WHEN** LLM 流式产出的工具调用不满足 `concurrency_safe`、参数尚未闭合或 identity 尚未稳定
- **THEN** 系统 SHALL 不提前执行该调用
- **AND** 它 SHALL 回退到完整 assistant 输出后的常规 tool cycle

#### Scenario: 候选在后续流式阶段失效时丢弃提前执行结果

- **WHEN** 一个已提前执行的工具候选在后续流式阶段被证明不再与 assistant 最终工具计划一致
- **THEN** 系统 SHALL 丢弃该候选的提前执行结果
- **AND** 该结果 SHALL NOT 进入 durable tool 事实流

#### Scenario: durable 顺序保持 assistant 先于工具事实

- **WHEN** 某个工具调用在流式阶段提前开始甚至提前完成
- **THEN** durable 事件写入顺序仍 SHALL 保持 assistant 定稿在前
- **AND** tool call / tool result 事实 SHALL 在该 assistant 定稿之后有序落盘
