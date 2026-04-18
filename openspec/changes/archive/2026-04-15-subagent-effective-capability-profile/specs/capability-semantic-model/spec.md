## ADDED Requirements

### Requirement: child capability surfaces SHALL remain projections of the current capability semantic model

子 agent 的可见能力面 MUST 从当前 `CapabilitySpec` / capability router 派生，而不是通过 `AgentProfile` 或其他平行配置对象重新定义一套能力真相。

#### Scenario: child launch resolves capability surface

- **WHEN** 系统启动一个 child agent
- **THEN** child 的能力面 MUST 基于当前 capability router 求得
- **AND** MUST NOT 仅凭 `AgentProfile` 直接决定 child 可调用哪些工具

#### Scenario: status and replay explain child capabilities

- **WHEN** 系统查询或回放某个 child 的运行状态
- **THEN** 用于解释该 child capability 的数据 MUST 对应到启动时的 capability semantic projection
- **AND** MUST NOT 退回到读取最新 profile 文件重新推断

### Requirement: task-scoped capability grants SHALL constrain runtime capability projections

面向 child agent 的任务级授权 MUST 作为 capability projection 的输入之一，而不是扩展 `AgentProfile` 成为权限对象。

#### Scenario: spawn requests a restricted capability grant

- **WHEN** 调用方在 `spawn` 时提供任务级 capability grant
- **THEN** 系统 MUST 用该 grant 约束 child 的 capability projection
- **AND** grant 的语义 MUST 建立在现有 `CapabilitySpec` 命名与分类之上

#### Scenario: no task-scoped capability grant is provided

- **WHEN** 调用方未提供 capability grant
- **THEN** child 的 capability projection MUST 回退到父级可继承上界与 runtime availability 的交集
- **AND** MUST NOT 因为缺少 grant 就去读取 profile 中的工具白名单作为替代真相
