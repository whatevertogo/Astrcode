## Why

Astrcode 现在已经有 `session_plan` 和 `plan mode`，但它们承担的是“正式计划工件”和“治理审批流程”的职责，不适合承载执行期的高频工作记忆。长任务里，模型只能从 transcript、tool 输出和历史思路里逆向推断“当前正在做什么”，容易出现偏题、重复劳动、遗漏未完成项和恢复后失忆。

现在补上独立的 task 系统，是因为 Plan 语义已经在 `application`、`session-runtime` 和前端读模型里稳定落地，边界足够清晰。此时再增加一个与 Plan 解耦、专门服务执行阶段的 task tool / projection / prompt 注入链路，可以直接提升长任务稳定性，而不会污染 canonical session plan 的治理语义。

## What Changes

- 新增内置执行期工具 `taskWrite`，用于让模型以全量快照方式维护当前执行清单，而不是改写 `session_plan`。
- 新增执行期 task 数据模型，首版采用 `content`、`status`、`activeForm` 的轻量结构，优先支持单个活跃任务和少量任务列表。
- 新增基于 durable tool result metadata 的 task 投影与恢复机制，让当前 task 状态可跨 turn、重连和 session 恢复。
- 在 `session-runtime` 的 turn/request 装配链路中动态注入当前 task 摘要，让同一 turn 后续步骤和下一轮执行都能读取最新执行清单。
- 扩展终端 / 前端 conversation read model，暴露独立 task panel 所需的 authoritative facts，但不把 task 与 Plan Surface 混成同一种 UI 语义。
- 限制 `taskWrite` 只在执行态可用；`plan` / `review` mode 继续只使用正式 plan 机制。
- 为未来演进预留空间，但本次不引入 Claude Code V2 风格的 `taskCreate` / `taskUpdate` / `taskList` / owner-blocking 文件任务板。

## Capabilities

### New Capabilities
- `execution-task-tracking`: 定义执行期 task tool、task snapshot 数据模型、作用域、持久化 / 恢复语义、prompt 注入语义，以及与 `session_plan` 的边界。

### Modified Capabilities
- `terminal-chat-read-model`: conversation snapshot / stream 需要暴露 active task panel 所需的 authoritative facts，前端不得通过本地重放 tool history 自行拼任务面板。
- `application-use-cases`: `application` 需要通过 `SessionRuntime` 的稳定 query/command 入口暴露 task display facts，而不是直接扫描底层 tool 事件或让上层自行组装。

## Impact

- `crates/core`：新增 task 相关稳定类型（`ExecutionTaskItem`、`ExecutionTaskStatus`、`TaskSnapshot`）与 metadata schema 结构，供 tool、runtime 和前端映射复用。
- `crates/adapter-tools`：新增 `taskWrite` builtin tool（`SideEffect::Local`），并提供 prompt guidance（`ToolPromptMetadata`）、输入校验（含 20 条上限）和 metadata 写出（`schema: “executionTaskSnapshot”`）。
- `crates/server`：在 bootstrap 能力装配中注册 `taskWrite`。
- `crates/session-runtime`：在 `SessionState` 新增 `active_tasks` 缓存字段，在 `translate_store_and_cache()` 中拦截 `taskWrite` 的 `ToolResult` 事件更新投影；在 `SessionQueries` 新增 `active_task_snapshot()` 查询方法；在 `turn/request` 的 `build_prompt_output()` 中新增 `live_task_snapshot_declaration`。
- `crates/application`：在 `terminal_control_facts()` 中通过 `SessionQueries::active_task_snapshot()` 读取 task facts，映射到 `TerminalControlFacts.active_tasks`。
- `crates/protocol`：在 `ConversationControlStateDto` 新增 `activeTasks` 字段。
- `frontend`：新增 task 卡片组件（对话区顶部折叠卡片），接入 conversation control state 的 hydration 和 `UpdateControlState` delta。
- 用户可见影响：执行阶段会出现独立 task 卡片（对话区顶部），模型会更稳定地维持当前工作清单。
- 开发者可见影响：需要遵守”task 是执行期状态，不是 session plan””durable truth 优先来自事件日志/工具结果投影”的边界。
- 依赖影响：本次不新增核心第三方依赖。

## Non-Goals

- 不把 `taskWrite` 与 `upsertSessionPlan` / `exitPlanMode` 复用同一状态文件或同一前端 surface。
- 不在首版引入 Claude Code V2 风格的任务 ID、owner、blockedBy、blocks、共享任务板或跨 session 任务目录。
- 不在首版新增专门的 `StorageEventPayload::TaskUpdated`；优先复用已有 tool result durable 事件与投影链路。
- 不要求客户端通过 transcript block 回放出 task 真相；task 卡片以 authoritative read model 为准。
- `taskWrite` 工具调用本身保留为正常 `ToolCallBlock`（不做抑制），与 plan 工具的抑制行为不同。
- 不要求同步修改 `PROJECT_ARCHITECTURE.md`；本次方案与现有 `application` / `session-runtime` / `adapter-*` 分层保持一致。
