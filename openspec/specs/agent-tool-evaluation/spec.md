## Purpose

定义 agent-tool 协作效果的评估体系，包括原始协作事实的记录、策略上下文捕获、稳定效果评分卡和可消费的评估读模型。

## Requirements

### Requirement: agent collaboration facts MUST be recorded as structured server-side records

系统 MUST 为 agent-tool 的关键协作动作记录结构化原始事实，并保证这些事实来自 server-side 业务真相，而不是只存在于前端或临时内存状态。

#### Scenario: collaboration actions occur

- **WHEN** 系统执行 `spawn`、`send`、`observe`、`close` 或 child delivery 相关流程
- **THEN** 系统 MUST 记录对应的结构化协作事实
- **AND** 这些事实 MUST 至少包含 parent、child、turn、动作类型和结果状态

#### Scenario: collaboration action fails

- **WHEN** 协作动作因限制、所有权错误或执行失败而未成功完成
- **THEN** 系统 MUST 记录失败事实
- **AND** MUST 保留可用于后续诊断的失败原因分类

### Requirement: evaluation records MUST capture effective policy context

协作评估记录 MUST 同时包含生效中的策略上下文，以支持不同 prompt/runtime 策略之间的比较。

#### Scenario: collaboration fact is recorded

- **WHEN** 系统写入一条 agent collaboration fact
- **THEN** 记录 MUST 包含该动作生效时的策略上下文或其稳定引用
- **AND** 该上下文 MUST 能表达深度限制、fan-out 限制与 prompt/policy revision 的等价信息

### Requirement: system MUST derive a stable effectiveness scorecard from raw facts

系统 MUST 基于原始协作事实生成稳定的诊断读模型，用于判断 agent-tool 是否创造了实际协作价值。

#### Scenario: scorecard is built

- **WHEN** 系统为某轮 turn 或某段运行窗口生成效果读模型
- **THEN** 读模型 MUST 能表达 child reuse、observe-to-action、spawn-to-delivery、orphan child 与 delivery latency 等核心指标
- **AND** MUST 明确区分"没有数据"与"结果为零"

#### Scenario: raw facts are incomplete

- **WHEN** 某些协作事实来源尚未接线或不可用
- **THEN** 读模型 MUST 显式反映该缺口
- **AND** MUST NOT 静默把缺失数据伪装成有效低值

### Requirement: evaluation read models MUST be consumable without replaying full transcripts

系统 MUST 提供稳定的评估读模型，避免开发者为了判断 agent-tool 效果而手工重扫整条 transcript 或原始事件流。

#### Scenario: developer reads collaboration effectiveness

- **WHEN** 开发者读取 turn 级或全局级的 agent-tool 效果信息
- **THEN** 系统 MUST 返回稳定聚合后的纯数据结构
- **AND** DTO MUST NOT 承载新的业务逻辑
