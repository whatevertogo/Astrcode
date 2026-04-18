## MODIFIED Requirements

### Requirement: Runtime observability SHALL cover read and execution paths

系统 MUST 同时采集读路径与执行路径的关键指标，包括 session rehydrate、SSE catch-up、turn execution、subrun execution、delivery diagnostics 以及 agent collaboration diagnostics。

#### Scenario: Read path metrics are recorded

- **WHEN** 系统执行 session 重水合或 SSE 回放
- **THEN** 对应 observability 指标 SHALL 被记录

#### Scenario: Execution path metrics are recorded

- **WHEN** 系统执行 turn、subrun、delivery 或 agent collaboration 相关流程
- **THEN** 对应 observability 指标 SHALL 被记录
- **AND** 失败路径同样 SHALL 被统计

#### Scenario: Collaboration diagnostics are exposed

- **WHEN** 上层读取治理快照或等价 observability 读模型
- **THEN** 返回结果 SHALL 包含 agent collaboration 诊断
- **AND** 该诊断 SHALL 能区分 spawn、send、observe、close、delivery 与拒绝/失败路径
