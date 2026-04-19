## MODIFIED Requirements

### Requirement: Runtime observability SHALL cover read and execution paths

系统 MUST 同时采集读路径与执行路径的关键指标，包括 session rehydrate、SSE catch-up、turn execution、subrun execution、delivery diagnostics 以及 agent collaboration diagnostics。此外，observability 管线 MUST 支持评测场景下的指标导出，将 turn 级指标写入评测结果而非仅推送到 SSE/frontend。

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

#### Scenario: 评测运行时指标可被评测运行器收集

- **WHEN** 评测运行器通过 server API 执行评测任务
- **THEN** 运行器 SHALL 能通过读取 JSONL 事件获取所有 turn 级 observability 数据
- **AND** 不需要额外的 API 端点或导出机制
- **AND** 评测 trace 提取器从 `PromptMetrics`、`CompactApplied` 等已有事件中提取所需指标

## ADDED Requirements

### Requirement: observability 指标 SHALL 在 JSONL 中保持完整可提取性

运行时写入的所有 observability 相关事件（`PromptMetrics`、`CompactApplied`、`SubRunStarted/Finished`）MUST 在 JSONL 中保持完整的字段信息，确保离线评测可以无损提取。

#### Scenario: PromptMetrics 包含完整 provider 指标

- **WHEN** provider 返回 token 使用统计和 cache 命中数据
- **THEN** `PromptMetrics` 事件 MUST 在 JSONL 中持久化所有 `PromptMetricsPayload` 字段
- **AND** 离线评测读取时 MUST 能无损恢复这些数据

#### Scenario: CompactApplied 包含完整的压缩效果数据

- **WHEN** 发生上下文压缩
- **THEN** `CompactApplied` 事件 MUST 持久化 `pre_tokens`、`post_tokens_estimate`、`messages_removed`、`tokens_freed` 字段
- **AND** 这些字段是评测 compaction 效率的 ground truth
