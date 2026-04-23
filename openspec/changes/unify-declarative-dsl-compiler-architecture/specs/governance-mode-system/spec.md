## ADDED Requirements

### Requirement: governance mode spec SHALL describe mode contracts beyond capability selection

`GovernanceModeSpec` MUST 能声明完整 mode 合同，而不只是 capability selector、action policy 与 child policy。该合同在本次至少覆盖：mode 级 artifact 定义、exit gate 与动态 prompt hooks。

#### Scenario: builtin plan mode declares its artifact contract through mode spec

- **WHEN** builtin `plan` mode 需要维护 canonical plan artifact
- **THEN** 系统 SHALL 通过 `GovernanceModeSpec` 的 mode contract 字段声明该 artifact 的 kind、写入口约束与退出前置条件
- **AND** SHALL NOT 只依赖 `upsertSessionPlan` / `exitPlanMode` 的硬编码约定表达这些语义

#### Scenario: plugin mode registers a complete mode contract

- **WHEN** 插件通过 `InitializeResultData.modes` 声明自定义 mode
- **THEN** 该 mode SHALL 可以同时声明 capability surface、artifact contract、exit gate 与 prompt hooks
- **AND** host SHALL 用与 builtin mode 相同的校验与编译流程消费该合同

### Requirement: compile and bind responsibilities SHALL remain explicitly separated in governance mode processing

mode processing MUST 维持“compile 产物”和“bound surface”两层边界。compile 阶段 SHALL 负责 selector 求值、mode contract 派生与 diagnostics；bind 阶段 SHALL 负责 runtime/session/profile/control 绑定，并生成最终可执行治理面。

#### Scenario: compiler derives mode contract without reading session runtime state

- **WHEN** 系统编译一个 `GovernanceModeSpec`
- **THEN** compile 阶段 SHALL 只依赖当前 capability semantic model、mode spec 与显式输入
- **AND** SHALL NOT 直接读取 session-runtime 的运行时状态来决定 artifact contract 或 exit gate 语义

#### Scenario: binder consumes compiled mode artifact to produce the final governance surface

- **WHEN** 系统在 root、session、fresh child 或 resumed child 入口解析治理面
- **THEN** binder SHALL 在已编译的 mode artifact 基础上绑定 runtime config、resolved limits、profile、injected messages 与 approval pipeline
- **AND** SHALL NOT 回流承担 selector 解释或 mode contract 语义校验

### Requirement: tool-consumable mode contracts SHALL be projected as pure data

凡是工具执行需要消费的 mode contract 语义，系统 MUST 通过纯数据的 bound mode contract snapshot 投影到工具上下文，而不是要求工具依赖 application 内部类型或自行重建治理语义。

#### Scenario: plan tools consume artifact and exit contract through tool context

- **WHEN** builtin `plan` 工具需要读取 artifact 写约束或 exit gate checklist
- **THEN** 系统 SHALL 在 tool / capability context 中提供 pure-data bound contract snapshot
- **AND** 工具 SHALL NOT 直接依赖 `GovernanceSurfaceAssembler`、`ModeCatalog` 或 session-runtime 内部状态来重建同类 contract

#### Scenario: capability bridge preserves the bound mode contract snapshot

- **WHEN** 工具上下文被桥接成 capability context 再回到 tool context
- **THEN** 该 pure-data bound contract snapshot SHALL 被稳定保留
- **AND** SHALL NOT 因桥接路径丢失 contract 语义

### Requirement: mode catalog SHALL reject duplicate stable IDs across builtin and plugin registries

mode catalog MUST 拒绝 duplicate `mode_id`。插件 mode SHALL NOT 覆盖 builtin `code` / `plan` / `review`，也 SHALL NOT 与其他 plugin mode 使用同一个稳定 id。

#### Scenario: plugin mode cannot shadow a builtin mode

- **WHEN** 某个插件声明 `mode_id = "plan"` 或其他已存在 builtin id
- **THEN** host SHALL 拒绝该候选治理快照
- **AND** 错误结果 SHALL 能指出冲突的 `mode_id`

#### Scenario: duplicate plugin mode ids are rejected before catalog swap

- **WHEN** 同一轮 bootstrap / reload 中两个 plugin mode 使用同一个 `mode_id`
- **THEN** 系统 SHALL 在 mode catalog 候选快照阶段拒绝该输入
- **AND** SHALL NOT 进入后续 capability surface 提交

## MODIFIED Requirements

### Requirement: governance mode SHALL compile to a turn-scoped execution envelope

> 修改自 `openspec/specs/governance-mode-system/spec.md` 中同名 requirement。
> 变更：envelope 编译结果现在包含 mode contract 派生的 artifact / exit / prompt 治理输入；
> workflow phase 绑定仍由 workflow artifact 持有，而不是反向塞进 mode spec。

系统 SHALL 在 turn 边界把当前 mode 编译为 turn-scoped 的治理执行包络。该编译结果 MUST 至少包含当前 turn 的 capability surface、prompt declarations、execution limits、action policies、child policy，以及 mode contract 派生出的 artifact / exit / prompt 相关治理输入。

#### Scenario: plan mode compiles a restricted capability surface through declarative mode contract

- **WHEN** 当前 session 的 mode 为一个规划型 mode
- **THEN** 系统 SHALL 为该 turn 编译出收缩后的 capability router
- **AND** 规划型 mode 的 selector SHALL 能排除 `SideEffect::Local`、`SideEffect::Workspace`、`SideEffect::External` 与 `Tag("agent")` 的工具，或通过等价组合表达式得到同等结果
- **AND** 若该 mode 需要额外的 artifact 写约束或 exit gate 语义，SHALL 通过 `ModeArtifactDef` 和 `ModeExitGateDef` 显式声明，而不是把具体工具名硬编码进 selector 或编译器
- **AND** 当前 turn 模型可见的工具集合 SHALL 与该 router 保持一致

#### Scenario: code mode compiles the full default envelope

- **WHEN** 当前 session 的 mode 为 builtin `code`
- **THEN** 系统 SHALL 编译出与当前默认执行行为等价的 envelope
- **AND** SHALL NOT 因引入 mode contract 而额外改变 turn loop 语义
