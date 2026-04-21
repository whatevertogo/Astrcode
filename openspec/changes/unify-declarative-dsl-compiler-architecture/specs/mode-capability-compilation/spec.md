## ADDED Requirements

### Requirement: CapabilitySelector evaluation SHALL remain the deterministic core of mode compilation

`CapabilitySelector` 的递归求值 MUST 继续作为 mode compiler 的核心算法，并 SHALL 对 root turn、child policy 裁剪与 capability grant 裁剪提供一致、可复用的选择语义。相同 selector 在相同 capability semantic model 上 MUST 产出相同结果。

#### Scenario: root mode compilation and child derivation reuse the same selector semantics

- **WHEN** 同一个 selector 同时用于 root mode 编译和 child policy 裁剪
- **THEN** 系统 SHALL 复用同一套 selector 求值语义
- **AND** SHALL NOT 在 child 派生路径上引入另一套与 root mode 不一致的筛选规则

#### Scenario: selector result remains stable across builtin and plugin capabilities

- **WHEN** 当前 capability surface 同时包含 builtin、MCP 与 plugin capabilities
- **THEN** selector evaluation SHALL 只基于 `CapabilitySpec` 字段求值
- **AND** SHALL NOT 因能力来源不同而改变并集、交集、差集的结果

### Requirement: mode compilation SHALL produce a reusable compiled capability projection before runtime binding

mode capability compilation MUST 先产出可复用的 compiled capability projection，再由 binder 将其绑定到具体 turn 上。该 compiled projection SHALL 表达 allowed tools、child capability projection、subset router 描述与编译期 diagnostics。

#### Scenario: compiler reports an empty projection before runtime submission

- **WHEN** 某个 mode 的 selector 编译结果为空
- **THEN** 编译阶段 SHALL 在 compiled projection 中记录诊断信息
- **AND** binder SHALL 继续消费该诊断，而不是在运行时重新猜测 selector 问题

#### Scenario: capability grant intersects after compiled projection is derived

- **WHEN** spawn 调用提供 `SpawnCapabilityGrant`
- **THEN** 系统 SHALL 先得到 mode 与 child policy 的 compiled capability projection
- **AND** 再与 grant 求交集得到 child 最终能力面
- **AND** SHALL NOT 让 grant 反向改变 mode compiler 对 selector 的基础解释
