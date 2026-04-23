## MODIFIED Requirements

### Requirement: system MUST derive a stable effectiveness scorecard from raw facts

系统 MUST 基于原始协作事实生成稳定的诊断读模型，用于判断 agent-tool 是否创造了实际协作价值。此外，协作评估的原始事实 MUST 可被评测 trace 提取器作为输入源，在评测场景中建立与 collaboration facts 的关联。

#### Scenario: scorecard is built

- **WHEN** 系统为某轮 turn 或某段运行窗口生成效果读模型
- **THEN** 读模型 MUST 能表达 child reuse、observe-to-action、spawn-to-delivery、orphan child 与 delivery latency 等核心指标
- **AND** MUST 明确区分"没有数据"与"结果为零"

#### Scenario: raw facts are incomplete

- **WHEN** 某些协作事实来源尚未接线或不可用
- **THEN** 读模型 MUST 显式反映该缺口
- **AND** MUST NOT 静默把缺失数据伪装成有效低值

#### Scenario: 协作事实被评测 trace 提取器消费

- **WHEN** 评测 trace 提取器处理包含 `AgentCollaborationFact` 事件的 JSONL
- **THEN** 提取器 MUST 将协作事实纳入 `TurnTrace` 的协作信息中
- **AND** 协作数据用于评估 agent delegation 的效果（如 spawn 成功率、delivery 延迟）

## ADDED Requirements

### Requirement: AgentCollaborationFact 事件 SHALL 在评测 trace 中可关联

`StorageEventPayload::AgentCollaborationFact` 中的协作事实 MUST 可在评测 trace 中与对应的工具调用、子 Agent 执行建立关联。

#### Scenario: 协作事实关联到子 Agent trace

- **WHEN** turn 内既有 `SubRunStarted/Finished` 也有 `AgentCollaborationFact`
- **THEN** 评测 trace 提取器 MUST 通过 `sub_run_id` 建立两者的关联
- **AND** 评测报告中子 Agent trace MUST 包含协作事实摘要
