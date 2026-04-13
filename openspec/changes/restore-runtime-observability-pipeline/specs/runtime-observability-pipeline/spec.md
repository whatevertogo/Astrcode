## ADDED Requirements

### Requirement: Runtime observability SHALL be backed by live collectors

系统 MUST 使用真实运行时采集器生成 observability 快照，而不是使用默认零值占位实现。

#### Scenario: Governance snapshot exposes live metrics

- **WHEN** 上层读取治理快照
- **THEN** 返回的 observability 数据 SHALL 来自实时采集器
- **AND** MUST NOT 以固定零值伪装为真实结果

#### Scenario: Missing metric source is explicit

- **WHEN** 某类指标来源尚未接线或暂时不可用
- **THEN** 系统 SHALL 显式处理该状态
- **AND** MUST NOT 静默返回误导性的成功零值

### Requirement: Runtime observability SHALL cover read and execution paths

系统 MUST 同时采集读路径与执行路径的关键指标，包括 session rehydrate、SSE catch-up、turn execution、subrun execution 和 delivery diagnostics。

#### Scenario: Read path metrics are recorded

- **WHEN** 系统执行 session 重水合或 SSE 回放
- **THEN** 对应 observability 指标 SHALL 被记录

#### Scenario: Execution path metrics are recorded

- **WHEN** 系统执行 turn、subrun 或 delivery 相关流程
- **THEN** 对应 observability 指标 SHALL 被记录
- **AND** 失败路径同样 SHALL 被统计
