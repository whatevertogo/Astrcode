## Purpose

确保 plugin 的发现、刷新与治理视图在每次 reload 与治理快照中均可追踪、可核验，并与统一能力集合保持一致。

## MODIFIED Requirements

### Requirement: Plugin Lifecycle Appears In Governance

系统 SHALL 将 plugin 的发现、装载、失败、刷新结果纳入治理视图与统一 reload 编排。

#### Scenario: Governance snapshot includes plugins

- **WHEN** 上层请求治理快照
- **THEN** 快照 SHALL 包含 plugin 相关状态
- **AND** 至少覆盖已发现、已装载、失败原因与参与当前生效 surface 的结果

#### Scenario: Governance snapshot reflects retained old state on reload failure

- **WHEN** 某次 reload 因 plugin 刷新失败而未完成 surface 替换
- **THEN** 治理快照 SHALL 继续反映旧的生效 surface 状态
- **AND** 同时暴露本次 plugin 刷新失败信息

#### Scenario: Reload reports plugin refresh result

- **WHEN** reload 完成
- **THEN** 系统 SHALL 能表达 plugin 刷新成功或失败的结果
- **AND** 该结果 SHALL 与当前 capability surface 状态一致

#### Scenario: Reload success reflects plugin participation

- **WHEN** plugin 刷新成功且新 surface 已生效
- **THEN** reload 结果 SHALL 反映 plugin 参与新 surface 的状态

#### Scenario: Reload failure does not claim new surface

- **WHEN** plugin 刷新失败导致统一 surface 未替换
- **THEN** reload 结果 MUST NOT 声称新 plugin surface 已生效
