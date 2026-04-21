## MODIFIED Requirements

### Requirement: Plugin Capabilities Participate In Unified Surface

系统 SHALL 允许 plugin 提供的 capabilities、skills、hooks 通过统一物化链路并入同一个运行时候选快照：capabilities 进入 capability surface，skills 进入 skill catalog，hooks 进入 hooks registry。三者 MUST 在 bootstrap 与 reload 时一起参与候选构建、提交与回滚，而不是各自独立切换。

#### Scenario: Bootstrap loads plugin capabilities

- **WHEN** 系统启动并发现可用 plugin
- **THEN** 组合根 SHALL 装载 plugin 并物化其能力描述
- **AND** 这些能力 SHALL 能参与 `kernel` capability surface 的构建
- **AND** plugin 提供的 hooks SHALL 同时参与 hooks registry 的初始构建

#### Scenario: Reload refreshes plugin surface participation

- **WHEN** 系统执行 reload
- **THEN** plugin 能力、skills 与 hooks SHALL 重新发现、重新物化并参与同一候选快照替换
- **AND** SHALL NOT 只停留在 plugin manager 的内部缓存中
- **AND** SHALL 与 builtin、MCP 一起形成统一候选 surface / registry 状态

#### Scenario: Plugin failure is visible

- **WHEN** 某个 plugin 装载失败或能力/skill/hook 物化失败
- **THEN** 系统 SHALL 在治理视图中暴露失败信息
- **AND** SHALL 继续保持整体 surface / registry 的一致性
- **AND** SHALL NOT 让失败 plugin 把系统推进到半刷新状态
