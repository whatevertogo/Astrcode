## Purpose

定义模型可见的 child delegation surface，明确 delegation catalog、child-scoped execution contract 与 capability-aware 限制提示应该如何呈现。

## Requirements

### Requirement: model-visible delegation catalog SHALL expose behavior templates, not capability authority

当当前 session 可使用 child delegation 时，系统 MUST 提供一个面向模型的 delegation catalog，并且该 catalog 只能基于当前可作为 child 使用的 behavior template 生成；它 MUST NOT 把 `AgentProfile` 伪装成 capability 授权目录。

#### Scenario: render available child templates

- **WHEN** prompt builder 为具备 child delegation 能力的 session 组装 delegation surface
- **THEN** 系统 MUST 只展示当前可作为 child 使用的 behavior template
- **AND** 每个 entry MUST 包含足以帮助选择的行为模板摘要或用途说明

#### Scenario: hide unavailable child templates

- **WHEN** 某个 profile 当前不允许作为 child，或当前 runtime / policy 不允许模型使用对应 delegation 能力
- **THEN** 该 profile MUST NOT 出现在 delegation catalog 中
- **AND** 系统 MUST NOT 指望 runtime 在模型选择之后再去纠正一个本可提前隐藏的 entry

#### Scenario: catalog does not claim per-profile tool ownership

- **WHEN** 系统渲染 delegation catalog
- **THEN** catalog MUST NOT 把某个 behavior template 表达成一组静态工具权限
- **AND** MUST 保持“profile 是行为模板，capability truth 在 launch 时求解”的边界

### Requirement: delegation surface SHALL reflect the resolved governance envelope

模型可见的 child delegation catalog 与 child-scoped execution contract MUST 受当前 turn 的 resolved governance envelope 约束，而不是只根据静态 profile 列表或全局默认行为生成。

#### Scenario: delegation catalog is omitted when current mode forbids child delegation

- **WHEN** 当前 turn 的 governance envelope 禁止创建新的 child 分支
- **THEN** 系统 SHALL 不渲染可供选择的 child delegation catalog
- **AND** SHALL NOT 让模型先看到不可用条目再依赖 runtime 事后拒绝

#### Scenario: governance envelope narrows visible child templates

- **WHEN** 当前 mode 只允许一部分 behavior template 用于 child delegation
- **THEN** delegation catalog SHALL 仅展示这些允许的 template
- **AND** SHALL 继续保持"profile 是行为模板，而非权限目录"的表达边界

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

### Requirement: child execution contracts SHALL be emitted from the shared governance assembly path

fresh child 与 resumed child 的 execution contract MUST 由统一治理装配路径生成，而不是由不同调用路径分别手工拼接。

#### Scenario: fresh child contract uses the shared assembly path

- **WHEN** 系统首次启动一个承担新责任分支的 child
- **THEN** child execution contract SHALL 通过共享治理装配器生成
- **AND** SHALL 与同一次提交中的其他治理声明保持同一事实源

#### Scenario: resumed child contract uses the same authoritative source

- **WHEN** 父级复用已有 child 并发送 delta instruction
- **THEN** resumed child contract SHALL 由同一治理装配路径生成
- **AND** SHALL NOT 退回到独立 helper 拼接的平行实现

### Requirement: delegation catalog and child contracts SHALL stay consistent under the same governance surface

delegation catalog 可见的 behavior template、child execution contract 中的责任边界与 capability-aware 限制 MUST 来源于同一治理包络。

#### Scenario: catalog and contract agree on branch constraints

- **WHEN** 某个 child template 在当前提交中可见且被用于启动 child
- **THEN** delegation catalog 与最终 child execution contract SHALL 体现一致的责任边界和限制摘要
- **AND** SHALL NOT 让 catalog 与 contract 分别读取不同来源的治理事实

### Requirement: collaboration facts SHALL be recordable with governance envelope context

`AgentCollaborationFact`（`core/agent/mod.rs`）记录 spawn/send/observe/close/delivery 等协作动作的审计事件。这些事实 MUST 能关联到生成该动作时的治理包络上下文，使审计链路可追溯。

#### Scenario: collaboration fact includes governance context

- **WHEN** 系统记录一个 `AgentCollaborationFact`（如 spawn 或 send）
- **THEN** 该事实 SHALL 能关联到当前 turn 的治理包络标识或摘要
- **AND** SHALL NOT 丢失治理上下文导致无法追溯决策依据

#### Scenario: policy revision aligns with governance envelope

- **WHEN** `AGENT_COLLABORATION_POLICY_REVISION` 用于标记协作策略版本
- **THEN** 该版本标识 SHALL 与治理包络中的策略版本一致
- **AND** SHALL NOT 出现审计事实的策略版本与实际治理策略不同步

### Requirement: CollaborationFactRecord SHALL derive its parameters from the governance envelope

`CollaborationFactRecord`（`agent/context.rs`）跟踪每个协作动作的结果、原因码和延迟。其构建参数 MUST 来自治理包络，而不是各调用点独立组装。

#### Scenario: fact record uses governance-resolved child identity and limits

- **WHEN** 系统为一个 spawn 或 send 动作构建 `CollaborationFactRecord`
- **THEN** child identity、capability limits 等字段 SHALL 从治理包络中获取
- **AND** SHALL NOT 从不同参数源独立读取导致与治理包络不一致

### Requirement: child execution contract SHALL be rendered through a child-scoped prompt surface

系统 MUST 为 child agent 渲染独立的 execution contract prompt surface，用来明确责任边界、交付方式与限制条件，而不是要求调用方仅靠自然语言 prompt 自行约定这些信息。

#### Scenario: fresh child receives full execution contract

- **WHEN** 系统首次启动一个承担新责任边界的 child
- **THEN** child prompt MUST 包含该责任边界、期望交付形式与回传摘要要求
- **AND** 这些信息 MUST 作为 child-scoped contract surface 出现，而不是散落在工具 description 中

#### Scenario: resumed child receives delta-oriented execution contract

- **WHEN** 父级复用已有 child 并发送下一步任务
- **THEN** child prompt MUST 保留既有 responsibility continuity
- **AND** 新增 prompt 内容 MUST 以具体 delta instruction 为主，而不是重新灌入完整 fresh briefing

#### Scenario: restricted child receives capability-aware contract

- **WHEN** child 以收缩后的 capability surface 启动
- **THEN** child execution contract MUST 明确暴露本次 capability limit 的摘要
- **AND** MUST 明确 child 不应承担超出该 capability surface 的工作
