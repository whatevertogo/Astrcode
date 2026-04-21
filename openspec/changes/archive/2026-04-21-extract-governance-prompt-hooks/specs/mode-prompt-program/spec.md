## ADDED Requirements

### Requirement: dynamic mode prompt inputs SHALL resolve through governance prompt hooks

builtin mode 的运行时动态 prompt 输入 MUST 通过 governance prompt hooks 解析，而不是由 `session_use_cases` 或 mode-specific helper 直接在提交流程中拼接 prompt declarations。解析结果 SHALL 继续作为 `PromptDeclaration` 注入既有 prompt 组装路径。

#### Scenario: plan mode without an active artifact uses a mode-active hook

- **WHEN** 当前 session 处于 builtin `plan` mode，且没有 active plan artifact
- **THEN** 系统 SHALL 通过 `ModeActive` 类 governance prompt hook 生成 plan facts declaration
- **AND** SHALL 同时生成首次规划模板 declaration，而不是在提交主流程中手工拼接专用 helper

#### Scenario: plan mode with an active artifact uses a re-entry hook

- **WHEN** 当前 session 处于 builtin `plan` mode，且当前任务已有 active plan artifact
- **THEN** 系统 SHALL 通过 `ModeActive` 类 governance prompt hook 生成 plan facts declaration
- **AND** SHALL 额外生成 re-entry declaration，指导模型在同一 canonical plan 上继续修订

### Requirement: mode exit prompt overlays SHALL resolve through governance prompt hooks

mode 退出后的 prompt overlay MUST 通过 governance prompt hooks 生成，以便后续 mode contract 能统一复用退出提示，而不是继续把 plan 专属 exit prompt 硬编码在 session 提交流程中。

#### Scenario: approved plan emits exit prompt through a mode-exit hook

- **WHEN** 当前 session 的 active plan 被批准，且系统把 session 从 `plan` mode 切换回 `code` mode
- **THEN** 系统 SHALL 通过 `ModeExit` 类 governance prompt hook 生成 approved plan exit declaration
- **AND** 该 declaration SHALL 包含 approved artifact 的 path、slug、title 与 status
