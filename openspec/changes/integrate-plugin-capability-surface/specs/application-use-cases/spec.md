## MODIFIED Requirements

### Requirement: Application Governs Plugin Reload

`application` MUST 通过治理入口编排 plugin 参与的 reload 流程。

#### Scenario: Reload triggers full capability refresh

- **WHEN** 上层触发 reload
- **THEN** `application` SHALL 编排完整刷新链路
- **AND** 刷新结果 SHALL 同时覆盖 builtin、MCP、plugin 能力来源

#### Scenario: Governance does not hide plugin failure

- **WHEN** plugin 发现、装载或物化失败
- **THEN** `application` SHALL 暴露明确错误或治理快照结果
- **AND** SHALL NOT 静默吞掉失败

