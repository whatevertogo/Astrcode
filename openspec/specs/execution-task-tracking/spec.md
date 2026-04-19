# execution-task-tracking Specification

## Purpose
定义执行期 task（`taskWrite`）的完整生命周期规范，包括 snapshot 写入语义、ownership 隔离、durable 回放恢复、prompt 注入、mode 可见性约束与 prompt guidance。

## Requirements

### Requirement: execution tasks SHALL remain independent from the canonical session plan

系统 MUST 将执行期 task 与 canonical `session_plan` 视为两套不同真相。`taskWrite` 只用于维护当前执行清单，MUST NOT 读写 `sessions/<id>/plan/**`，也 MUST NOT 改变当前 `activePlan`、plan 审批状态或 Plan Surface 语义。

#### Scenario: taskWrite updates execution state without mutating session plan

- **WHEN** 模型在 `code` mode 调用 `taskWrite`
- **THEN** 系统 SHALL 仅更新执行期 task snapshot
- **AND** SHALL NOT 修改 session plan markdown、plan state 或 plan review 状态

#### Scenario: plan workflow remains the only formal planning path

- **WHEN** 模型需要产出、更新或呈递正式计划
- **THEN** 系统 SHALL 继续要求使用 `upsertSessionPlan` / `exitPlanMode`
- **AND** SHALL NOT 接受 `taskWrite` 作为 formal plan 的替代写入口

### Requirement: taskWrite SHALL accept a full execution-task snapshot

`taskWrite` MUST 接受当前 owner 的完整任务快照，而不是增量 patch。每个 task item MUST 至少包含 `content` 与 `status`，其中 `status` 只能是 `pending`、`in_progress` 或 `completed`。若某项为 `in_progress`，则该项 MUST 提供 `activeForm`。同一个 snapshot 中 MUST NOT 存在多个 `in_progress` 项。单次快照 MUST NOT 超过 20 条 item。

#### Scenario: valid snapshot is accepted

- **WHEN** `taskWrite` 收到一个合法的 task 列表，且至多只有一个 `in_progress` 项
- **THEN** 系统 SHALL 接受该调用
- **AND** SHALL 将该列表视为当前 owner 的最新完整 task snapshot

#### Scenario: invalid snapshot is rejected

- **WHEN** `taskWrite` 输入包含未知状态、多个 `in_progress` 项，或某个 `in_progress` 项缺少 `activeForm`
- **THEN** 系统 SHALL 拒绝该调用并返回明确错误
- **AND** SHALL NOT 落盘部分 task 状态

#### Scenario: oversized snapshot is rejected

- **WHEN** `taskWrite` 输入包含超过 20 条 item
- **THEN** 系统 SHALL 拒绝该调用并返回明确错误
- **AND** SHALL NOT 落盘部分 task 状态

### Requirement: task ownership SHALL be scoped to the current execution owner

系统 MUST 为 task snapshot 绑定稳定 owner。owner MUST 优先取当前工具上下文中的 `agent_id`；若该字段缺失，则 MUST 回退到当前 `session_id`。不同 owner 的 task snapshot MUST 相互隔离。

#### Scenario: root execution falls back to session ownership

- **WHEN** 一次 `taskWrite` 调用发生在没有 `agent_id` 的根执行上下文
- **THEN** 系统 SHALL 使用当前 `session_id` 作为 owner
- **AND** 后续读取时 SHALL 返回该 session 的最新 task snapshot

#### Scenario: child or agent-scoped task snapshots do not overwrite each other

- **WHEN** 两个不同 owner 分别写入 task snapshot
- **THEN** 系统 SHALL 为它们分别保留最新 snapshot
- **AND** 一个 owner 的写入 SHALL NOT 覆盖另一个 owner 的 task 状态

### Requirement: active task state SHALL be recoverable from durable tool results

执行期 task 真相 MUST 可通过 durable tool result 回放恢复。`taskWrite` 的最终结果 MUST 以结构化 metadata（`schema: "executionTaskSnapshot"`）形式持久化最新 snapshot。`SessionState` MUST 维护 `active_tasks: HashMap<String, TaskSnapshot>` 缓存（key = owner），在 `translate_store_and_cache()` 中拦截 `taskWrite` 的 `ToolResult` 事件并更新投影。session reload、replay 或重连后，系统 SHALL 通过完整事件回放恢复每个 owner 的最新 task 状态。空列表或"全部 completed"列表 MUST 使该 owner 的条目被移除。系统 MUST NOT 为 task 维护独立的持久化文件（task 真相完全来自 durable tool result）。

#### Scenario: latest task snapshot survives replay

- **WHEN** 某个 session 已经成功记录过一个 `taskWrite` tool result
- **THEN** 会话重载或回放后 SHALL 恢复该 owner 的最新 task snapshot
- **AND** 前端 hydration 与后续 prompt 注入 SHALL 看到相同的 active tasks

#### Scenario: completed or empty snapshot clears active tasks

- **WHEN** `taskWrite` 写入空列表，或写入一个全部为 `completed` 的列表
- **THEN** 系统 SHALL 将该 owner 视为没有 active tasks
- **AND** 后续 prompt 注入与 task panel SHALL 隐藏该 owner 的 task 状态

### Requirement: active tasks SHALL be injected into subsequent turn steps

系统 MUST 在 turn request assembly 的 `build_prompt_output()` 中将当前 owner 的 active tasks 作为动态 prompt facts 注入，让同一 turn 的后续步骤也能读取最新执行清单。注入内容 MUST 只包含 `in_progress` 和 `pending` 状态的项，MUST NOT 回放 `completed` 项或历史快照。当前 owner 无 active tasks 时，MUST NOT 生成 task prompt 声明。注入 MUST 使用独立 block_id（如 `"task.active_snapshot"`），不影响其他 prompt declaration。

#### Scenario: a taskWrite call influences later steps in the same turn

- **WHEN** 模型在某个 turn 中调用 `taskWrite`，随后同一 turn 还会继续请求模型
- **THEN** 后续 step 的 prompt SHALL 包含该 owner 最新的 active task 摘要
- **AND** 模型 SHALL 不需要等到下一轮 turn 才能看到刚写入的 task 状态

#### Scenario: cleared tasks are removed from later prompts

- **WHEN** 当前 owner 的最新 task snapshot 为空或全部 completed
- **THEN** 后续 step 的 prompt SHALL 不再包含 active task 声明
- **AND** 系统 SHALL 不继续向模型暗示已完成的执行清单

### Requirement: taskWrite SHALL only be available in execution-oriented modes

`taskWrite` MUST 声明 `SideEffect::Local`，通过现有 mode selector 集合代数自动限制可见性。`plan` mode 和 `review` mode MUST NOT 向模型暴露该工具。Code mode（`AllTools`）MUST 包含 `taskWrite`。

#### Scenario: code mode exposes taskWrite

- **WHEN** 当前 session 处于 builtin `code` mode 或等价执行态 mode
- **THEN** capability surface SHALL 包含 `taskWrite`
- **AND** 模型可通过该工具维护执行期 task snapshot

#### Scenario: plan and review modes hide taskWrite

- **WHEN** 当前 session 处于 `plan` 或 `review` mode
- **THEN** capability surface SHALL 不包含 `taskWrite`（因为 `SideEffect::Local` 被 plan mode 排除、review mode 只允许 `SideEffect::None`）
- **AND** 模型 MUST 继续分别使用 formal plan 工具或只读审查工具

### Requirement: taskWrite SHALL carry detailed prompt guidance

`taskWrite` 的 capability metadata MUST 包含 `ToolPromptMetadata`，提供以下使用指导：
- 何时主动使用：3+ 步骤任务、用户提供多个任务、非平凡多步操作。
- 何时不用：单步简单任务、纯对话查询、3 步内可完成的琐碎操作。
- 状态管理规则：同一时刻最多 1 个 `in_progress`；开始工作前先标 `in_progress`；完成后立即标 `completed`。
- 双形式要求：每项必须同时提供 `content`（祈使句）和 `activeForm`（进行时）。
- 完成标准：只在真正完成时标 `completed`；测试失败或实现部分时保持 `in_progress`。
