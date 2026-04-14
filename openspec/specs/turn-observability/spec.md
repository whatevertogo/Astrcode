## Purpose

统一 turn 级 observability 的稳定汇总输出，减少治理端对原始事件流重建的依赖。

## Requirements

### Requirement: turn 执行形成稳定的 observability 汇总

`session-runtime` SHALL 为每个 turn 产生稳定的执行汇总结果，至少覆盖耗时、continuation 次数、prompt cache reuse、compaction 命中情况以及 agent collaboration summary，并将这些结果汇入运行时 observability 管线。

#### Scenario: turn 成功完成时生成汇总

- **WHEN** turn 正常结束
- **THEN** 系统产生一份包含成功状态与关键指标的 turn 汇总
- **AND** 该汇总 SHALL 能被运行时 observability 读取和累计

#### Scenario: turn 失败或取消时生成汇总

- **WHEN** turn 因错误或取消结束
- **THEN** 系统仍然生成 turn 汇总
- **AND** 汇总中体现失败或取消状态
- **AND** 失败结果 SHALL 汇入运行时 observability

#### Scenario: turn includes child collaboration

- **WHEN** 某轮 turn 内发生 `spawn`、`send`、`observe`、`close`、delivery 消费或 child reuse
- **THEN** turn 汇总 SHALL 包含 collaboration summary
- **AND** 该 summary SHALL 至少覆盖动作计数、拒绝/失败情况和关键延迟或等价效率信息

### Requirement: 原始事件与聚合结果职责分离

`PromptMetrics`、`CompactApplied` 以及 agent collaboration facts 等事件 SHALL 继续作为原始事实源，但治理和诊断读取 SHALL 使用聚合后的稳定结果，而不是重复扫描整条事件流。

#### Scenario: cache reuse 由原始事件汇入稳定结果

- **WHEN** turn 期间产生带 cache reuse 指标的 `PromptMetrics`
- **THEN** turn 汇总反映该 cache reuse 信息
- **AND** 运行时 observability SHALL 通过稳定聚合结果累计该指标

#### Scenario: compaction 命中由原始事件汇入稳定结果

- **WHEN** turn 期间发生 compact 或 reactive compact
- **THEN** turn 汇总反映该次 compact 命中信息
- **AND** 治理读取 SHALL 基于聚合结果而不是临时重扫事件流

#### Scenario: collaboration facts 由原始事件汇入稳定结果

- **WHEN** turn 期间产生 agent collaboration facts
- **THEN** turn 汇总 SHALL 基于这些事实生成 collaboration summary
- **AND** 治理读取 SHALL 基于该 summary 而不是直接重扫原始协作事件

### Requirement: turn 汇总 SHALL 暴露结构化 transition 与 stop cause

`session-runtime` 产生的 turn 级稳定汇总 MUST 暴露足够的结构化 loop 原因信息，至少包括“最后一次 transition 原因”和“最终 stop cause”，从而让治理与诊断路径不必重新扫描或猜测 turn loop 的推进方式。

#### Scenario: turn 通过 transition 继续后完成

- **WHEN** turn 期间经历了一次或多次显式 transition 后正常结束
- **THEN** turn 汇总 SHALL 反映最后一次 transition 原因
- **AND** 汇总 SHALL 同时反映最终 stop cause

#### Scenario: turn 在 budget stop 下结束

- **WHEN** turn 因 budget stop cause 结束
- **THEN** turn 汇总 SHALL 明确体现该 stop cause
- **AND** observability 读取方 MUST NOT 通过扫描原始消息历史来推断此结论

#### Scenario: turn 在 reactive compact 后重试

- **WHEN** turn 通过 reactive compact 路径完成恢复并继续推进
- **THEN** 稳定汇总 SHALL 能区分这次推进来自恢复路径而不是普通工具回合

### Requirement: 输出截断恢复 SHALL 进入稳定 turn 汇总

当 turn 通过输出截断 continuation 恢复继续推进时，`session-runtime` 产生的稳定 turn 汇总 MUST 反映恢复次数、是否因达到恢复上限而停止，以及最终 stop cause。

#### Scenario: turn 经历至少一次截断恢复

- **WHEN** turn 在执行期间发生至少一次输出截断恢复
- **THEN** turn 汇总 SHALL 记录恢复次数
- **AND** 治理读取方 SHALL 能区分该 turn 与普通自然结束 turn

#### Scenario: 因恢复上限停止

- **WHEN** turn 因达到输出截断恢复上限而停止
- **THEN** turn 汇总 SHALL 明确体现该停止原因
- **AND** observability MUST NOT 把该 turn 误判为普通自然结束
