## ADDED Requirements

### Requirement: mode SHALL compile to action policies that the PolicyEngine enforces

每个 governance mode MUST 编译为 action policies，作为 `ResolvedTurnEnvelope` 的一部分。`PolicyEngine` 的三个检查点 SHALL 在 turn 执行链路中消费这些 action policies。

#### Scenario: execute mode compiles permissive action policies

- **WHEN** 当前 session 的 mode 为 builtin `code`
- **THEN** action policies SHALL 编译为默认允许所有能力调用
- **AND** `check_capability_call` SHALL 返回 `PolicyVerdict::Allow`（与当前 `AllowAllPolicyEngine` 行为等价）

#### Scenario: plan mode compiles restrictive action policies

- **WHEN** 当前 session 的 mode 为 builtin `plan`
- **THEN** action policies SHALL 禁止具有 `SideEffect::Workspace` 或 `SideEffect::External` 的能力调用
- **AND** `check_capability_call` SHALL 对这些调用返回 `PolicyVerdict::Deny`

#### Scenario: custom mode compiles ask-on-high-risk policies

- **WHEN** 一个插件 mode 定义了"高风险操作需审批"的 action policy
- **THEN** 对高风险能力调用 `check_capability_call` SHALL 返回 `PolicyVerdict::Ask`
- **AND** 系统 SHALL 发起审批流（通过治理包络建立的管线）

### Requirement: PolicyContext SHALL be populated from the mode-compiled envelope

`PolicyContext`（core/policy/engine.rs:108-124）的构建 MUST 从 mode 编译后的治理包络派生，确保策略引擎与 turn 执行链路使用同一事实源。

#### Scenario: PolicyContext session/turn identifiers come from envelope

- **WHEN** PolicyEngine 需要构建 `PolicyContext` 用于裁决
- **THEN** session_id、turn_id、step_index、working_dir、profile SHALL 从治理包络中获取
- **AND** SHALL NOT 在调用点独立组装

#### Scenario: PolicyContext profile aligns with envelope capability surface

- **WHEN** mode 编译后的 envelope 指定了特定的 capability surface
- **THEN** PolicyContext 可用的能力信息 SHALL 与 envelope 一致
- **AND** SHALL NOT 出现 PolicyContext 认为某工具可用但 envelope 已移除的不一致

### Requirement: mode SHALL influence context strategy decisions

`decide_context_strategy`（PolicyEngine 的上下文策略检查点）SHALL 能参考当前 mode 的上下文治理偏好，使不同 mode 可以有不同的 context pressure 处理策略。

#### Scenario: execute mode uses default context strategy

- **WHEN** context pressure 触发策略裁决且当前 mode 为 `code`
- **THEN** 策略 SHALL 使用默认的 `ContextStrategy::Compact`（与当前行为等价）

#### Scenario: review mode prefers truncate over compact

- **WHEN** context pressure 触发策略裁决且当前 mode 为 `review`
- **THEN** 策略 MAY 优先使用 `ContextStrategy::Truncate` 而非 Compact
- **AND** SHALL NOT 丢失 review 对象的内容

#### Scenario: mode does not specify context strategy

- **WHEN** mode spec 未定义上下文策略偏好
- **THEN** 系统 SHALL 使用 runtime config 的默认策略
- **AND** SHALL NOT 因缺少 mode 配置而无法裁决

### Requirement: mode-specific policy engine SHALL be swappable without modifying turn loop

mode 变更后，后续 turn 的策略行为 SHALL 通过替换治理包络中的 action policies 实现，MUST NOT 要求修改 `run_turn`、tool cycle 或 streaming path。

#### Scenario: mode transition changes policy behavior at turn boundary

- **WHEN** session 从 `code` mode 切换到 `plan` mode
- **THEN** 下一 turn 的 PolicyEngine 行为 SHALL 基于 plan mode 的 action policies
- **AND** 当前 turn 的执行 SHALL 不受影响（next-turn 生效语义）

#### Scenario: plugin mode provides custom policy implementation

- **WHEN** 一个插件 mode 定义了自定义的策略裁决逻辑
- **THEN** 系统 SHALL 通过治理包络中的 action policies 传递该逻辑
- **AND** SHALL NOT 要求插件直接实现 `PolicyEngine` trait 或修改 turn runner
