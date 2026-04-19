## ADDED Requirements

### Requirement: application SHALL expose task display facts through stable session-runtime contracts

在 conversation snapshot、stream catch-up 或等价的 task display 场景中，`application` MUST 通过 `SessionRuntime` 的 `SessionQueries::active_task_snapshot()` 稳定 query 方法读取 authoritative task facts，并在 `terminal_control_facts()` 中将结果映射为 `TerminalControlFacts.active_tasks` 字段。`application` MUST NOT 直接扫描原始 `taskWrite` tool 事件、手写 replay 逻辑或把待上层再拼装的底层事实当成正式合同向上传递。

#### Scenario: server requests conversation facts with active tasks

- **WHEN** `server` 请求某个 session 的 conversation snapshot 或 stream catch-up，且该 session 当前存在 active tasks
- **THEN** `application` SHALL 通过 `terminal_control_facts()` 返回已收敛的 task display facts
- **AND** `server` 只负责 DTO 映射（`to_conversation_control_state_dto()` 将 `active_tasks` 映射为 `ConversationControlStateDto.activeTasks`）、HTTP 状态码与 SSE framing

#### Scenario: application does not reconstruct tasks from raw tool history

- **WHEN** `application` 需要返回某个 session 的 active-task panel facts
- **THEN** 它 SHALL 统一通过 `SessionQueries::active_task_snapshot()` 读取结果
- **AND** SHALL NOT 自行遍历原始 tool result 或重写底层 projection 规则

#### Scenario: no active tasks yields None

- **WHEN** `application` 查询 task facts，但当前 session 无 active tasks（空列表或全部 completed）
- **THEN** `TerminalControlFacts.active_tasks` SHALL 为 `None`
- **AND** `ConversationControlStateDto.activeTasks` SHALL 为 `None`
