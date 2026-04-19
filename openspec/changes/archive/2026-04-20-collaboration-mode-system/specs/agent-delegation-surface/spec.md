## ADDED Requirements

### Requirement: delegation surface SHALL reflect the resolved governance envelope

模型可见的 child delegation catalog 与 child-scoped execution contract MUST 受当前 turn 的 resolved governance envelope 约束，而不是只根据静态 profile 列表或全局默认行为生成。

#### Scenario: delegation catalog is omitted when current mode forbids child delegation

- **WHEN** 当前 turn 的 governance envelope 禁止创建新的 child 分支
- **THEN** 系统 SHALL 不渲染可供选择的 child delegation catalog
- **AND** SHALL NOT 让模型先看到不可用条目再依赖 runtime 事后拒绝

#### Scenario: governance envelope narrows visible child templates

- **WHEN** 当前 mode 只允许一部分 behavior template 用于 child delegation
- **THEN** delegation catalog SHALL 仅展示这些允许的 template
- **AND** SHALL 继续保持“profile 是行为模板，而非权限目录”的表达边界

### Requirement: child execution contract SHALL include governance-derived branch constraints

child execution contract MUST 体现启动该 child 时生效的 governance child policy，包括 child 初始 mode、capability-aware 约束与是否允许继续委派。

#### Scenario: fresh child contract includes initial mode summary

- **WHEN** 系统首次启动一个新的 child session
- **THEN** child execution contract SHALL 明确该 child 当前使用的治理模式或等价治理摘要
- **AND** SHALL 说明该分支的责任边界与允许动作

#### Scenario: restricted child contract includes delegation boundary

- **WHEN** child 由当前 governance mode 以受限 delegation policy 启动
- **THEN** child execution contract SHALL 明确该 child 不应承担超出当前治理边界的工作
- **AND** SHALL 在需要更宽能力面或更宽 delegation 权限时要求回退到父级重新决策

### Requirement: DelegationMetadata SHALL reflect mode-compiled child policy

`DelegationMetadata`（responsibility_summary、reuse_scope_summary、restricted、capability_limit_summary）MUST 由 mode 编译的 child policy 驱动生成，而不是由局部 helper 独立构建。

#### Scenario: restricted flag comes from mode child policy

- **WHEN** 当前 mode 的 child policy 指定 child 为 restricted delegation
- **THEN** `DelegationMetadata.restricted` SHALL 为 true
- **AND** responsibility_summary 和 capability_limit_summary SHALL 反映 child policy 的约束

#### Scenario: reuse scope aligns with mode delegation constraints

- **WHEN** mode 限制 child reuse 的条件
- **THEN** `DelegationMetadata.reuse_scope_summary` SHALL 体现 mode 定义的复用边界
- **AND** SHALL NOT 使用与 mode 无关的默认复用策略

### Requirement: SpawnCapabilityGrant SHALL be derived from mode capability selector and child policy

child 的 `SpawnCapabilityGrant.allowed_tools` MUST 由 mode 的 capability selector 与 child policy 联合计算，而不是从 spawn 参数直接构造。

#### Scenario: grant is intersection of mode selector and spawn parameters

- **WHEN** mode 的 child policy 指定了 capability selector，同时 spawn 调用传入了 allowed_tools
- **THEN** 最终 `SpawnCapabilityGrant.allowed_tools` SHALL 为两者交集
- **AND** 空交集 SHALL 导致 spawn 被拒绝并返回明确错误

#### Scenario: mode with no child policy uses spawn parameters directly

- **WHEN** mode 未指定 child policy 的 capability selector
- **THEN** `SpawnCapabilityGrant` SHALL 使用 spawn 调用传入的 allowed_tools
- **AND** 行为与当前默认等价

### Requirement: delegation catalog SHALL be filtered by mode child policy

`AgentProfileSummaryContributor` 渲染的 child profile 列表 MUST 受 mode child policy 约束。mode 可以限制可用于 delegation 的 profile 范围。

#### Scenario: mode limits available profiles

- **WHEN** mode 的 child policy 仅允许部分 profile 用于 delegation
- **THEN** delegation catalog SHALL 仅展示这些允许的 profile
- **AND** 不可用 profile SHALL 不出现在列表中

#### Scenario: mode forbids delegation entirely

- **WHEN** mode 的 child policy 禁止所有 delegation
- **THEN** spawn 工具 SHALL 不在可见能力面中
- **AND** `AgentProfileSummaryContributor` SHALL 因 spawn 不可用而不渲染（通过现有守卫条件自动生效）
