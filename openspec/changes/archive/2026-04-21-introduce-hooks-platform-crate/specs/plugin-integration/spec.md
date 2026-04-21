## MODIFIED Requirements

### Requirement: Hook → Plugin 适配

server SHALL 将 plugin 声明的 lifecycle hooks 适配为 `astrcode-hooks` 平台中的 external handlers，而不是继续只围绕 `core::HookHandler` 的窄版工具/compact hook trait 建模。plugin hooks MUST 通过 hooks 平台的事件、typed input、effect 约束与执行报告模型参与运行时执行。

#### Scenario: 注册 Hook

- **WHEN** 插件声明了 `PreToolUse`、`PermissionRequest`、`BeforeTurnSubmit` 或其他受支持的 lifecycle hook
- **THEN** 系统 SHALL 将该声明物化为 hooks 平台中的 external handler
- **AND** SHALL 把它注册到统一 hooks registry，而不是只创建 `core::HookHandler` 适配器

#### Scenario: Hook 执行

- **WHEN** turn 执行、权限裁决、compact 或 session/subagent 生命周期触发对应 hook 事件
- **THEN** 适配器 SHALL 通过 plugin 的 JSON-RPC peer 调用插件 hook handler
- **AND** 返回结果 SHALL 先按 hooks 平台的 schema 解析，再由 application 解释为合法 effect

## ADDED Requirements

### Requirement: plugin hook effects SHALL be constrained by the hooks platform and governance boundaries

plugin hooks MUST 服从 hooks 平台的 event-scoped effect 约束，并受 governance / policy / capability surface 的硬边界保护。plugin hooks SHALL NOT 通过 allow 类 effect 绕过原本已拒绝的策略裁决或放大工具权限。

#### Scenario: plugin permission hook cannot override a hard deny

- **WHEN** 插件的 `PermissionRequest` hook 对一个已被 governance 或 policy 拒绝的动作返回 allow
- **THEN** 系统 SHALL 忽略该放大权限的 effect
- **AND** SHALL 记录一条可观测的 hook diagnostics

#### Scenario: plugin before-turn hook can add context without bypassing prompt pipeline

- **WHEN** 插件的 `BeforeTurnSubmit` hook 返回额外上下文或 prompt 相关 effect
- **THEN** 系统 SHALL 通过 hooks 平台将其收敛为合法 prompt declarations
- **AND** SHALL 继续走既有治理装配与 prompt 组装路径
