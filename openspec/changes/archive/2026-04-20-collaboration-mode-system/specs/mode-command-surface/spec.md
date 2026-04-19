## ADDED Requirements

### Requirement: mode switching SHALL be accessible through a /mode slash command

用户 MUST 能通过 `/mode` slash 命令切换当前 session 的 governance mode。该命令 SHALL 集成到现有 `Command` enum（cli/src/command/mod.rs）中。

#### Scenario: /mode with no argument shows current mode and available modes

- **WHEN** 用户输入 `/mode`（无参数）
- **THEN** 系统 SHALL 显示当前 session 的 mode 和 catalog 中可用的 mode 列表
- **AND** 显示内容 SHALL 包括每个 mode 的名称和简短描述

#### Scenario: /mode with mode ID switches to target mode

- **WHEN** 用户输入 `/mode plan`
- **THEN** 系统 SHALL 校验 `plan` 是有效的 mode ID
- **AND** 校验通过后 SHALL 在当前 turn 追加 mode 变更事件
- **AND** 新 mode SHALL 从下一次 turn 开始生效

#### Scenario: /mode with invalid mode ID is rejected

- **WHEN** 用户输入 `/mode nonexistent`
- **THEN** 系统 SHALL 返回错误提示，列出可用的 mode ID
- **AND** SHALL NOT 改变当前 mode

### Requirement: /mode SHALL support tab completion from the mode catalog

`/mode` 命令 SHALL 支持 tab 补全，补全候选来自当前 mode catalog 中可用的 mode ID。

#### Scenario: tab completion lists available modes

- **WHEN** 用户在 `/mode ` 后按 tab
- **THEN** 系统 SHALL 显示当前 catalog 中所有可用 mode ID 作为候选
- **AND** 候选列表 SHALL 排除当前已处于的 mode

#### Scenario: tab completion filters by prefix

- **WHEN** 用户输入 `/mode pl` 后按 tab
- **THEN** 系统 SHALL 过滤并显示以 "pl" 开头的 mode ID（如 "plan"）
- **AND** 若无匹配 SHALL 不显示候选

### Requirement: /mode SHALL integrate with the existing slash command palette

`/mode` 命令 SHALL 出现在 slash command palette 中，与 `/model`、`/compact` 等命令并列。

#### Scenario: /mode appears in slash candidates

- **WHEN** 用户输入 `/` 触发 slash palette
- **THEN** `/mode` SHALL 出现在候选列表中
- **AND** SHALL 附带描述文本（如 "Switch execution governance mode"）

### Requirement: mode command SHALL route through unified application governance entrypoint

`/mode` 命令的解析和执行 MUST 走统一的 application 治理入口，而不是在 `session-runtime` 中解析命令语法。这与项目架构中 "slash command 只是输入壳，语义映射到稳定 server/application contract" 的要求一致。

#### Scenario: CLI sends mode transition request to application layer

- **WHEN** CLI 收到 `/mode plan` 命令
- **THEN** 它 SHALL 将 mode transition 请求发送到 application 的统一治理入口
- **AND** application 层 SHALL 校验 target mode、entry policy 和 transition policy
- **AND** session-runtime SHALL 只接收已验证的 transition command

#### Scenario: mode transition from tool call uses the same governance path

- **WHEN** 模型通过工具调用请求 mode 切换
- **THEN** 该请求 SHALL 走与 `/mode` 命令相同的治理入口
- **AND** 校验逻辑 SHALL 完全一致

### Requirement: mode status SHALL be visible to the user

用户 MUST 能在 UI/CLI 中看到当前 session 的 active mode。

#### Scenario: session status shows current mode

- **WHEN** session 处于活跃状态
- **THEN** CLI/UI SHALL 显示当前 session 的 mode ID（如 `[plan]` 或 `[code]`）
- **AND** mode 变更后 SHALL 即时更新显示

#### Scenario: mode change is reported to the user

- **WHEN** mode 切换成功
- **THEN** 系统 SHALL 向用户确认 mode 已变更
- **AND** SHALL 提示新 mode 在下一 turn 生效

### Requirement: mode transition rejection SHALL provide actionable feedback

当 mode 切换被拒绝时，系统 MUST 提供清晰的错误信息和可操作的建议。

#### Scenario: transition policy violation is explained

- **WHEN** 当前 mode 的 transition policy 禁止切换到目标 mode
- **THEN** 系统 SHALL 解释拒绝原因（如 "Cannot switch from review to plan: transition not allowed"）
- **AND** SHALL 列出从当前 mode 可以切换到的 mode 列表

#### Scenario: running session blocks certain mode transitions

- **WHEN** 某些 mode 要求在无 running turn 时才能切换
- **THEN** 系统 SHALL 提示用户等待当前 turn 完成后再切换
- **AND** SHALL NOT 静默忽略切换请求
