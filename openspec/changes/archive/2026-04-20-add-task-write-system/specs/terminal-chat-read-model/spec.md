## ADDED Requirements

### Requirement: conversation surface SHALL expose authoritative active-task panel facts

`conversation` surface 的 hydration snapshot 与增量 stream MUST 直接暴露当前 session 的 active-task panel facts，通过 `ConversationControlStateDto.activeTasks: Option<Vec<TaskItemDto>>` 字段传递。该事实源 MUST 来自服务端 authoritative projection（`SessionState.active_tasks` → `terminal_control_facts()` → DTO 映射），客户端 MUST NOT 通过扫描 `taskWrite` tool history、metadata fallback 或本地 reducer 自行重建任务面板。

#### Scenario: hydration snapshot includes current active tasks

- **WHEN** 终端或前端首次打开一个 session，并且该 session 当前存在 active tasks
- **THEN** 服务端 SHALL 在 conversation hydration 结果的 `activeTasks` 字段中返回当前 active-task panel facts
- **AND** 客户端 MUST 能在不回放历史 tool result 的前提下直接渲染任务卡片

#### Scenario: stream delta updates the task panel after taskWrite

- **WHEN** 当前 session 成功写入新的 `taskWrite` snapshot
- **THEN** conversation 增量流 SHALL 通过 `UpdateControlState` delta 推送更新后的 `activeTasks`
- **AND** 客户端 MUST 能仅凭该 authoritative delta 更新任务卡片

#### Scenario: task panel hides after tasks are cleared

- **WHEN** 当前 session 的最新 task snapshot 为空或全部 completed
- **THEN** `activeTasks` SHALL 为 `None`
- **AND** 客户端 SHALL 隐藏 task 卡片

#### Scenario: taskWrite tool calls appear as normal ToolCallBlocks

- **WHEN** 模型在 transcript 中调用 `taskWrite`
- **THEN** 该调用 SHALL 作为正常 `ToolCallBlock` 出现在消息列表中
- **AND** 系统 SHALL NOT 抑制该工具调用（与 plan 工具被 `should_suppress_tool_call_block()` 抑制的行为不同）
- **AND** `activeTasks` control state 与 transcript 中的 ToolCallBlock 是两个独立的 UI 表面
