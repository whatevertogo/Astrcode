## ADDED Requirements

### Requirement: Plugin Capabilities Participate In Unified Surface

系统 SHALL 允许 plugin 提供的 capabilities、skills、hooks 通过统一物化链路并入 capability surface。

#### Scenario: Bootstrap loads plugin capabilities

- **WHEN** 系统启动并发现可用 plugin
- **THEN** 组合根 SHALL 装载 plugin 并物化其能力描述
- **AND** 这些能力 SHALL 能参与 `kernel` capability surface 的构建

#### Scenario: Reload refreshes plugin surface participation

- **WHEN** 系统执行 reload
- **THEN** plugin 能力 SHALL 重新发现、重新物化并参与整份 surface 替换
- **AND** SHALL NOT 只停留在 plugin manager 的内部缓存中

#### Scenario: Plugin failure is visible

- **WHEN** 某个 plugin 装载失败或能力物化失败
- **THEN** 系统 SHALL 在治理视图中暴露失败信息
- **AND** SHALL 继续保持整体 surface 的一致性

