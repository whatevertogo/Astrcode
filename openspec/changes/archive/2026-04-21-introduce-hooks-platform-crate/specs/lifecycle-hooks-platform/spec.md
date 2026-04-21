## ADDED Requirements

### Requirement: hooks platform SHALL live in an independent crate and provide one shared protocol for builtin and external hooks

系统 MUST 提供独立的 `astrcode-hooks` crate，作为 Astrcode 生命周期 hooks 的正式平台。该平台 SHALL 为 builtin hooks 与 external hooks 提供同一套事件名、typed input、effect、matcher、registry、runner 与执行报告模型，而不是让内置系统与插件扩展分别走私有实现。

#### Scenario: builtin and external hooks share one registry model

- **WHEN** 系统注册 builtin plan/workflow hooks 与 plugin 提供的 lifecycle hooks
- **THEN** 它们 SHALL 进入同一 hooks registry 抽象
- **AND** SHALL 由同一个 runner 负责匹配、执行与报告

#### Scenario: core no longer owns the authoritative hooks platform

- **WHEN** Astrcode 需要扩展新的 lifecycle hook 事件或 effect
- **THEN** authoritative 协议 SHALL 定义在 `astrcode-hooks` crate 中
- **AND** `core::hook` SHALL NOT 继续作为唯一事实源承载平台演进

### Requirement: core SHALL retain only a minimal shared semantic surface for hooks

`core` MAY 保留 hooks 相关的极小共享语义类型或兼容壳层，但 SHALL NOT 拥有 hooks 平台的 registry、runner、matcher、reload、report、schema 或执行顺序/失败语义。hooks 平台依赖 `core` 的稳定语义，而不是反过来把完整平台写回 `core`。

#### Scenario: core exposes only compatibility or shared semantic types

- **WHEN** 其他 crates 仍需要从 `core` 引用历史 hook 名称或少量共享语义
- **THEN** `core` MAY 提供兼容再导出或极小语义类型
- **AND** SHALL NOT 在 `core` 中重新实现完整 hooks 平台运行时

#### Scenario: hooks platform runtime remains outside core

- **WHEN** 系统新增新的 hook 匹配规则、执行器、reload 逻辑或 observability 报告能力
- **THEN** 这些能力 SHALL 落在 `astrcode-hooks` 或更外层消费模块
- **AND** SHALL NOT 以“共享语义”为由重新回流到 `core`

### Requirement: hooks platform SHALL expose typed events and event-scoped effects

hooks 平台 MUST 提供稳定的 typed 事件与 event-scoped effect 约束。第一阶段至少 SHALL 支持 `SessionStart`、`SessionEnd`、`BeforeTurnSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`PermissionDenied`、`PreCompact`、`PostCompact`、`SubagentStart`、`SubagentStop`。

#### Scenario: before-turn hooks add prompt declarations

- **WHEN** `BeforeTurnSubmit` hook 在 turn 提交前运行并产出 prompt 相关 effect
- **THEN** 系统 SHALL 只接受与 turn-level context 补充相关的 effect
- **AND** 这些 effect SHALL 可被转换为 `PromptDeclaration` 并进入既有 prompt 组装链路

#### Scenario: permission hooks cannot widen a denied policy decision

- **WHEN** `PermissionRequest` hook 试图对一个原本已被 policy 或 governance 硬性拒绝的动作返回 allow 类 effect
- **THEN** 系统 SHALL 拒绝该放大权限的 effect
- **AND** hooks SHALL 只能收紧、补充或在 ask 边界内帮助裁决

### Requirement: hook points SHALL be categorized into constrained lifecycle extension classes

hooks 平台 MUST 将 hook points 视为受约束的 lifecycle extension pipeline，而不是任意事件总线。每个 hook point SHALL 至少归属于 `observe`、`guard` 或 `augment` 之一，并且只允许该类别对应的 effect 集合。系统默认 SHALL NOT 开放可任意突变 session / turn / workflow 真相的 mutation hooks。

#### Scenario: observe hook cannot mutate governance truth

- **WHEN** `PostToolUse` 或 `PostCompact` 这类 observe hook 运行
- **THEN** 系统 SHALL 只接受 report、annotation、system note 等观察类 effect
- **AND** SHALL NOT 接受直接修改 session/workflow 真相的 mutation effect

#### Scenario: augment hook stays within prompt and context supplementation

- **WHEN** `BeforeTurnSubmit` hook 命中并返回 augment 类 effect
- **THEN** 系统 SHALL 只接受 prompt/context/system message 补充
- **AND** SHALL NOT 让该 hook 直接决定 turn 状态机推进或 workflow phase truth

### Requirement: hooks platform SHALL execute deterministically with explicit abort semantics

hooks runner MUST 以稳定顺序执行命中的 hooks，并为每次执行提供显式的继续、中止、失败继续和失败中止语义。相同输入在相同注册顺序下 SHALL 产生等价的执行顺序与 effect 合并顺序。

#### Scenario: blocking pre-tool hook aborts the pending tool call

- **WHEN** 某个 `PreToolUse` hook 返回 block/abort 类 effect
- **THEN** 当前工具调用 SHALL 不被执行
- **AND** 后续 effect 合并 SHALL 以该次中止为准，不再继续执行需要依赖该工具结果的后续路径

#### Scenario: multiple matching hooks keep stable merge order

- **WHEN** 多个 hooks 同时命中同一 `BeforeTurnSubmit` 事件
- **THEN** runner SHALL 按稳定注册顺序执行并合并 effect
- **AND** 重复运行相同输入时 SHALL 得到等价的 declaration / system message 顺序

### Requirement: hooks platform SHALL emit structured observability reports without becoming durable truth

每次 hook 执行 MUST 生成结构化执行报告，至少包含事件名、handler 来源、handler 类型、执行结果、耗时与 effect 摘要。hook execution 本身 SHALL NOT 被视为 session durable truth，也 SHALL NOT 在恢复时按历史记录重放。

#### Scenario: hook execution is visible in observability

- **WHEN** 某次 turn 触发多个 hooks
- **THEN** 系统 SHALL 为每次 hook 执行生成可观测报告
- **AND** 报告 SHALL 区分命中、跳过、成功、失败继续、失败中止等状态

#### Scenario: session recovery does not replay historical hooks

- **WHEN** 系统从 checkpoint 或 event log 恢复 session
- **THEN** 它 SHALL 根据恢复后的当前真相重新决定后续何时触发 hooks
- **AND** SHALL NOT 为了重建 session truth 而重放历史 hook execution

### Requirement: hooks platform SHALL support inline, command, and http handlers in the first phase

第一阶段 hooks 平台 MUST 支持 `inline`、`command`、`http` 三类 handler。builtin hooks SHALL 至少可使用 `inline`；external hooks SHALL 至少可通过 `command` 或 `http` 适配接入。

#### Scenario: builtin workflow overlay runs as inline hook

- **WHEN** 内置 plan/workflow overlay 需要在 `BeforeTurnSubmit` 事件上运行
- **THEN** 系统 SHALL 允许其作为 builtin inline hook 注册
- **AND** SHALL 不要求内置逻辑绕道 shell 或 plugin 进程

#### Scenario: external validation hook runs as command or http handler

- **WHEN** 外部扩展需要在 `PreToolUse` 或 `SessionStart` 等事件上介入
- **THEN** 系统 SHALL 允许其通过 `command` 或 `http` handler 接入
- **AND** SHALL 使用统一的 hooks runner、input schema 与 effect 解释规则
