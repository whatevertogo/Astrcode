## MODIFIED Requirements

### Requirement: Application Governs Plugin Reload

`application` MUST 通过治理入口编排完整 capability reload 流程，而不是只编排 plugin 自身刷新。

#### Scenario: Reload triggers full capability refresh

- **WHEN** 上层触发 reload
- **THEN** `application` SHALL 编排完整刷新链路
- **AND** 刷新结果 SHALL 同时覆盖 builtin、MCP、plugin 能力来源
- **AND** SHALL 以统一治理结果表达当前生效 surface

#### Scenario: Governance does not hide plugin failure

- **WHEN** plugin 发现、装载、物化或参与统一 surface 替换失败
- **THEN** `application` SHALL 暴露明确错误或治理快照结果
- **AND** SHALL NOT 静默吞掉失败
- **AND** SHALL NOT 让部分 plugin 刷新结果伪装成完整 reload 成功
