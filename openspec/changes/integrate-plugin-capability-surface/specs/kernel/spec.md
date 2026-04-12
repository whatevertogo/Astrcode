## MODIFIED Requirements

### Requirement: Kernel Replaces Unified Capability Surface

`kernel` MUST 支持用统一能力面一次性替换当前 surface。

#### Scenario: Builtin, MCP and plugin capabilities share one surface

- **WHEN** 组合根收集 builtin、MCP、plugin 三类能力来源
- **THEN** `kernel` SHALL 用单一 surface 替换入口接收它们
- **AND** SHALL 保证替换后的 surface 对外一致可见

#### Scenario: Partial plugin refresh is not enough

- **WHEN** plugin manager 内部状态变化但 `kernel` 未替换整份 surface
- **THEN** 系统 SHALL 视该刷新为不完整实现

