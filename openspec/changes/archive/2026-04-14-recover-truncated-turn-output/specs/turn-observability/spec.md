## ADDED Requirements

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

