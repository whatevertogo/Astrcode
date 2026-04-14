## ADDED Requirements

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

