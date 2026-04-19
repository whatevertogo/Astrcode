## Purpose

定义 governance mode 如何通过 CapabilitySelector 从当前 CapabilitySpec / capability router 投影出 mode-specific 的能力子集，编译为 scoped CapabilityRouter。

## Requirements

### Requirement: mode SHALL compile to a scoped CapabilityRouter through CapabilitySelector resolution

每个 governance mode MUST 通过 `CapabilitySelector` 从当前 `CapabilitySpec` / capability router 投影出 mode-specific 的能力子集，编译为 scoped `CapabilityRouter`，并在 turn 开始时通过 `scoped_gateway()` 固定工具面。

#### Scenario: execute mode compiles the full capability surface

- **WHEN** 当前 session 的 mode 为 builtin `code`
- **THEN** envelope 编译 SHALL 产生包含当前全部可见工具的 capability router
- **AND** `scoped_gateway()` (runner.rs:339) SHALL 传入该 router，结果与当前默认行为等价

#### Scenario: plan mode compiles a read-only capability subset

- **WHEN** 当前 session 的 mode 为 builtin `plan`
- **THEN** envelope 编译 SHALL 基于 CapabilitySelector 筛除具有 `SideEffect::Workspace` 或 `SideEffect::External` 的工具
- **AND** 保留 `SideEffect::None` 和 `SideEffect::Local` 的工具
- **AND** 模型在该 turn 中 SHALL NOT 看到或调用被筛除的工具

#### Scenario: review mode compiles an observation-only subset

- **WHEN** 当前 session 的 mode 为 builtin `review`
- **THEN** envelope 编译 SHALL 仅保留无副作用的工具（可能包括 observe、read-only 工具）
- **AND** SHALL 移除 spawn、send、close 等协作工具

### Requirement: CapabilitySelector SHALL resolve against current CapabilitySpec metadata

CapabilitySelector 的投影 MUST 严格基于当前 `CapabilitySpec` 的字段（name、kind、side_effect、tags），MUST NOT 维护平行的工具注册表。

#### Scenario: NameSelector matches exact capability name

- **WHEN** mode 使用 `Name("shell")` selector
- **THEN** 编译结果 SHALL 包含名称为 "shell" 的 capability（如果它在当前 surface 中存在）
- **AND** SHALL NOT 匹配名称不包含 "shell" 的 capability

#### Scenario: KindSelector matches capability kind

- **WHEN** mode 使用 `Kind(Tool)` selector
- **THEN** 编译结果 SHALL 包含所有 `CapabilityKind::Tool` 类型的 capability

#### Scenario: SideEffectSelector matches side effect level

- **WHEN** mode 使用 `SideEffect(None)` selector
- **THEN** 编译结果 SHALL 仅包含 `side_effect == SideEffect::None` 的 capability
- **AND** SHALL NOT 包含 `SideEffect::Local` 或更高级别的 capability

#### Scenario: TagSelector matches capability tags

- **WHEN** mode 使用 `Tag("source:mcp")` selector
- **THEN** 编译结果 SHALL 包含 tags 中含有 `"source:mcp"` 的 capability

#### Scenario: selector operates uniformly on builtin and plugin capabilities

- **WHEN** 当前 capability surface 同时包含 builtin 与插件工具
- **THEN** CapabilitySelector SHALL 对它们一视同仁地解析
- **AND** SHALL NOT 因来源不同而走不同选择路径

### Requirement: mode SHALL support compositional capability selectors

mode 的能力选择 SHALL 支持组合操作（并集、交集、差集），使 mode spec 能表达复杂的能力面约束。

#### Scenario: mode uses intersection of selectors

- **WHEN** mode 定义能力选择为 `Kind(Tool) ∩ NotSideEffect(External)`
- **THEN** 编译结果 SHALL 包含所有 Tool 类型且不具有 External 副作用的 capability

#### Scenario: mode uses exclusion selector

- **WHEN** mode 定义能力选择为 `AllTools - Name("spawn")`
- **THEN** 编译结果 SHALL 包含除 "spawn" 外的所有工具

### Requirement: child capability router SHALL be derived from parent mode's child policy

child session 的能力路由 MUST 由父 mode 的 child policy 推导，而不是简单继承父 session 的完整能力面。推导过程 SHALL 复用 CapabilitySelector 机制。

#### Scenario: child policy specifies narrower capability selector

- **WHEN** 父 mode 的 child policy 定义了 `capability_selector` 限制 child 可用工具
- **THEN** child 的 capability router SHALL 先按 child policy 的 selector 从父能力面中筛选
- **AND** SHALL NOT 直接继承父的完整能力面

#### Scenario: child policy intersects with SpawnCapabilityGrant

- **WHEN** 父 mode 的 child policy 有 capability selector，同时 spawn 调用指定了 `SpawnCapabilityGrant`
- **THEN** child 的最终能力面 SHALL 为两者交集
- **AND** 空交集 SHALL 导致 spawn 被拒绝
