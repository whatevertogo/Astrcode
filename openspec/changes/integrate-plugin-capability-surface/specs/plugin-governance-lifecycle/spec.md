## ADDED Requirements

### Requirement: Plugin Lifecycle Appears In Governance

系统 SHALL 将 plugin 的发现、装载、失败、刷新结果纳入治理视图与 reload 编排。

#### Scenario: Governance snapshot includes plugins

- **WHEN** 上层请求治理快照
- **THEN** 快照 SHALL 包含 plugin 相关状态
- **AND** 至少覆盖已发现、已装载、失败原因与参与 surface 的结果

#### Scenario: Reload reports plugin refresh result

- **WHEN** reload 完成
- **THEN** 系统 SHALL 能表达 plugin 刷新成功或失败的结果
- **AND** 该结果 SHALL 与当前 capability surface 状态一致

