## ADDED Requirements

### Requirement: subagent launch SHALL accept task-scoped capability grants

`application` 在 `spawn` child 时 MUST 支持任务级 capability grant，用于描述该 child 在本次任务里申请的最小能力范围，而不是要求调用方通过 `AgentProfile` 预组合出所有权限变体。

#### Scenario: spawn child with explicit capability grant

- **WHEN** 调用方在 `spawn` 时显式提供 capability grant
- **THEN** 系统 MUST 将该 grant 作为 child launch 的输入之一
- **AND** child 的运行时能力面 MUST 受该 grant 约束

#### Scenario: spawn child without capability grant

- **WHEN** 调用方未提供 capability grant
- **THEN** 系统 MUST 回退到父级可继承 capability surface 与 runtime availability 的交集
- **AND** MUST NOT 读取 profile 中的工具清单作为 child 权限真相

### Requirement: child resolved capability surface SHALL be derived from runtime capability truth

child 的 resolved capability surface MUST 由当前 capability router、父级可继承上界、任务级 grant 与 runtime availability 求交得到，而不是由 `AgentProfile` 直接决定。

#### Scenario: grant requests a subset of parent capabilities

- **WHEN** grant 申请的工具集合是父级当前可继承工具面的真子集
- **THEN** child 的 resolved capability surface MUST 收缩到该子集
- **AND** child MUST NOT 获得 grant 之外的额外工具

#### Scenario: grant requests unavailable or non-inheritable capability

- **WHEN** grant 申请了当前 runtime 不可用或父级不可继承的能力
- **THEN** 系统 MUST 在 child launch 时给出明确结果
- **AND** MUST NOT 悄悄让 child 获得超出上界的能力

#### Scenario: profile remains a behavior template

- **WHEN** child launch 解析目标 `AgentProfile`
- **THEN** profile SHALL 继续决定 child 的行为模板、模型偏好或默认 prompt 风格
- **AND** SHALL NOT 单独作为 child 工具授权的最终真相

### Requirement: child prompt assembly and tool execution SHALL share one filtered capability view

子代理的 prompt 组装与 tool execution MUST 基于同一份 filtered capability router / capability view，而不是分别读取不同来源的工具列表。

#### Scenario: prompt hides filtered-out tools

- **WHEN** child 的 resolved capability surface 过滤掉某些工具
- **THEN** child prompt 中的 capability 列表与 tool guidance MUST 不再暴露这些工具
- **AND** 这些工具的说明 MUST 随之从 child 的系统提示词可见面移除

#### Scenario: runtime rejects filtered-out tools

- **WHEN** child 后续规划到一个不在 resolved capability surface 内的工具
- **THEN** runtime MUST 将其视为不可用工具
- **AND** MUST NOT 因为全局 capability registry 中存在该工具就允许调用成功

#### Scenario: prompt and runtime remain aligned

- **WHEN** child 使用解析后的 capability surface 启动
- **THEN** prompt 中可见工具集合与 runtime 可执行工具集合 MUST 保持一致
- **AND** 系统 MUST NOT 出现“prompt 可见但 runtime 不可调”或“prompt 不可见但 runtime 可调”的漂移
