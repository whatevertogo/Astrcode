## Requirements

### Requirement: turn 执行形成稳定的 observability 汇总

`session-runtime` SHALL 为每个 turn 产生稳定的执行汇总结果，至少覆盖耗时、continuation 次数、prompt cache reuse 和 compaction 命中情况。

#### Scenario: turn 成功完成时生成汇总

- **WHEN** turn 正常结束
- **THEN** 系统产生一份包含成功状态与关键指标的 turn 汇总

#### Scenario: turn 失败或取消时生成汇总

- **WHEN** turn 因错误或取消结束
- **THEN** 系统仍然生成 turn 汇总
- **AND** 汇总中体现失败或取消状态

---

### Requirement: 原始事件与聚合结果职责分离

`PromptMetrics`、`CompactApplied` 等事件 SHALL 继续作为原始事实源，但治理和诊断读取 SHALL 使用聚合后的稳定结果，而不是重复扫描整条事件流。

#### Scenario: cache reuse 由原始事件汇入稳定结果

- **WHEN** turn 期间产生带 cache reuse 指标的 `PromptMetrics`
- **THEN** turn 汇总反映该 cache reuse 信息

#### Scenario: compaction 命中由原始事件汇入稳定结果

- **WHEN** turn 期间发生 compact 或 reactive compact
- **THEN** turn 汇总反映该次 compact 命中信息
