## Context

Astrcode 当前已经有一套明确的正式计划系统：

- `application::session_plan` 维护 session 下唯一 canonical plan artifact、审批、归档和 prompt 注入。
- `upsertSessionPlan` / `exitPlanMode` 是正式 plan 的唯一写入和呈递路径。
- `session-runtime` conversation projection 会把 plan 工具结果专门投影成 `Plan` block。
- 前端已经有 `PlanMessage`、`PlanSurface` 和 `activePlan` 控制态。

这说明 Astrcode 现有的 `plan` 语义不是“给模型记待办”，而是“正式治理工件”。因此，本次设计不能把 Todo/Task 简化成 `session_plan` 的轻量版。

对比现有三个参照系统，可以得到更清晰的定位：

| 系统 | 核心对象 | 主要回答的问题 | 生命周期 | 真相形态 |
| --- | --- | --- | --- | --- |
| Codex | `update_plan` / turn todo | 这一轮接下来做哪几步 | `per-turn` | thread item / turn 级投影 |
| Claude Code V1 | `TodoWrite` | 当前 agent 正在做什么 | `per-session` | AppState + 日志恢复 |
| Claude Code V2 | `TaskCreate/Get/Update/List` | 团队 / 多 agent 如何协作推进任务 | `跨会话` | 文件任务库 |
| Astrcode 当前 | `session_plan` | 这个 session 正式批准要做什么 | `session 级` | canonical artifact + 审批归档 |

Astrcode 缺的不是“更重的 plan”，而是“与 formal plan 分离的 execution task layer”。

另外，当前 turn request 装配有一个很关键的事实：`build_prompt_output()` 会在每一步重新调用 `PromptFactsProvider.resolve_prompt_facts()`，并在 `turn/request` 里追加 live declaration。这意味着如果 task 真要帮助模型“同一 turn 里也不忘事”，它必须走动态读事实链路，而不是只像 `session_plan` 那样在 submit 时塞一次固定声明。

本次变更与 `PROJECT_ARCHITECTURE.md` 不冲突，不需要先改架构总文档。原因是：

- `adapter-tools` 只负责工具定义与 capability 桥接；
- `session-runtime` 继续持有单 session task 真相与投影；
- `application` 继续只做稳定用例和治理编排；
- `frontend` 只消费 authoritative read model，不自己组 transcript 推断任务面板。

## Goals / Non-Goals

**Goals:**

- 引入与 `session_plan` 明确解耦的执行期 task 系统。
- 让模型能通过 `taskWrite` 外化当前执行清单，并在同一 turn 后续步骤与下一轮 turn 继续读取最新 task 状态。
- 保持 durable truth 优先来自已有事件日志 / tool result，而不是前端本地状态或新的平行存储。
- 提供单独 task panel 读模型，不污染 `PlanMessage` / `PlanSurface` 语义。
- 首版尽量轻量：只实现最小闭环 `taskWrite + projection + prompt 注入 + panel`。

**Non-Goals:**

- 不实现 Claude Code V2 风格的 task CRUD 工具族、任务依赖图和共享任务库。
- 不把 task 系统做成新的治理模式，也不在 `plan` / `review` mode 中开放写能力。
- 不为 task 单独新增一套 HTTP/SSE 专属 surface；优先复用现有 conversation/control read model 扩展。
- 不把 task 当作 transcript 主体消息类型；首版不要求历史回放里显示每次 task 变更记录。
- 不要求多 owner 并行面板、任务提醒器、快捷键等产品增强在首版一起落地。

## Decisions

### Decision 1：Task 系统与 Session Plan 严格分层

**选择：**

- `session_plan` 继续表示“正式计划、审批和归档”。
- 新增 `taskWrite` 表示“执行期清单和当前工作记忆”。
- `taskWrite` 不读写 `sessions/<id>/plan/**`，也不复用 `PlanMessage` / `activePlan`。

**理由：**

- Astrcode 已经把 plan 语义做成治理工件，如果再承载执行期清单，会把 canonical artifact 与临时执行状态混成一层。
- task 的更新频率、恢复语义和 UI 呈现都与 plan 完全不同。

**替代方案：**

- 复用 `upsertSessionPlan`：拒绝。语义错误，且会破坏 plan mode 的审查/审批边界。

### Decision 2：首版只做一个 `taskWrite`，采用全量快照模型

**选择：**

- 新增单一 builtin tool：`taskWrite`。
- 输入为全量 task 快照：
  - `content: string`
  - `status: pending | in_progress | completed`
  - `activeForm?: string`
- 首版要求最多一个 `in_progress` 任务。
- `in_progress` 项若存在，必须提供 `activeForm`，用于 prompt 和 UI 表达当前进行中的动作。
- 单次快照最多 20 条 item，超出时工具返回错误。防止大列表导致 token 膨胀。
- `taskWrite` 的 capability metadata 声明 `side_effect: Local`，这样 plan mode（排除 `SideEffect(Local)`）和 review mode（只允许 `SideEffect(None)`）自然不暴露该工具，code mode（`AllTools`）可见。无需在 mode catalog 中按名字添加白名单或黑名单。

**理由：**

- 全量快照最容易让模型理解当前完整状态，避免增量更新带来的 task id 管理和部分失败合并问题。
- Astrcode 现在的主要诉求是增强执行记忆，不是做团队任务板。
- `SideEffect::Local` 复用现有 mode selector 的集合代数，不需要为 `taskWrite` 特殊修改 mode catalog。

**替代方案：**

- 直接上 `taskCreate` / `taskUpdate` / `taskList`：拒绝。复杂度和多 agent 约束管理都过早。
- 只接受自由文本 checklist：拒绝。难以验证、难以投影、难以做稳定 prompt 注入。
- 使用 `SideEffect::None`：拒绝。会导致 taskWrite 在 plan 和 review mode 也可见，必须额外在 mode catalog 中按名字排除。

### Decision 3：Task 所有权采用 `agent_id || session_id`，但 UI 只显示当前 session 的 active snapshot

**选择：**

- task snapshot 写入时绑定 owner：
  - 优先使用 `ToolContext.agent_context().agent_id`
  - 若 agent id 缺失，则回退到当前 `session_id`
- `session-runtime` 为每个 owner 维护最新 snapshot。
- conversation/control read model 首版只暴露“当前 session 当前 owner 的 active tasks”。

**理由：**

- 这与 Claude Code V1 的“agentId ?? sessionId”隔离原则一致，同时兼容 Astrcode 现有 child session / subrun 体系。
- 当前 Astrcode 的 child 执行主要是独立 session，但预留 owner 维度可避免未来共享 session 分支时返工。

**替代方案：**

- 整个 session 只有一份任务表：接受度低。未来一旦出现共享 session 多 owner，就会污染任务真相。

### Decision 4：首版不新增专门事件类型，复用 durable tool result metadata

**选择：**

- `taskWrite` 正常走现有 `ToolCall` / `ToolResult` durable 事件链路。
- `ToolExecutionResult.metadata` 写入结构化 payload，例如：
  - `schema: "executionTaskSnapshot"`
  - `owner`
  - `items`
  - `cleared`
- `session-runtime` 新增 task projector，从 durable tool result 中提取最新 snapshot。
- `SessionState` 新增 `active_tasks: StdMutex<HashMap<String, TaskSnapshot>>` 字段（key = owner），遵循现有 `child_nodes: StdMutex<HashMap<String, ChildSessionNode>>` 和 `input_queue_projection_index` 模式。
- `SessionState::translate_store_and_cache()` 在处理 `StorageEventPayload::ToolResult` 时，若 `tool_name == "taskWrite"`，从 `metadata` 提取 snapshot 并更新对应 owner 的 `active_tasks` 条目。空列表或全部 completed 时移除该 owner 的条目。
- 冷启动（`SessionActor::from_replay()`）通过完整事件回放自动恢复所有 owner 的最新 task snapshot，不需要额外的持久化文件。
- `SessionState` 暴露查询方法 `active_tasks_for(owner: &str) -> Option<TaskSnapshot>`，供 prompt 注入和读模型使用。

**理由：**

- 这与当前系统的工具模型完全一致，接入成本低。
- 在需求尚未扩展到共享任务板前，没有必要提前引入新的 `StorageEventPayload::TaskUpdated`。
- `StdMutex<HashMap<String, TaskSnapshot>>` 与现有 `child_nodes` / `input_queue_projection_index` 保持一致的缓存模式。

**替代方案：**

- 新增专门 durable 事件：暂不采用。可以作为后续演进点，但首版没有必要扩大底层事件面。
- 前端内存保存任务状态：拒绝。恢复、回放和 prompt 注入都会失真。
- 独立持久化文件（类似 session plan state.json）：拒绝。task 真相已经由 durable tool result 保证，额外文件是冗余写入且增加一致性问题。

### Decision 5：动态 prompt 注入放在 `session-runtime/turn/request`，而不是 submit-time declaration

**选择：**

- 不沿用 `session_plan` 的 submit-time extra prompt declaration 方式。
- 在 `session-runtime::turn::request::build_prompt_output()` 中，参照 `live_direct_child_snapshot_declaration(...)`（当前位于 lines 313-317），新增 `live_task_snapshot_declaration(...)`。
- 每一步 request assembly 都从 `SessionState.active_tasks_for(current_owner)` 读取最新 snapshot 并生成精简 task 声明。
- 声明只包含活跃任务摘要（`in_progress` + `pending`），已完成项不注入，空列表不生成声明。
- 注入格式参照 `live_direct_child_snapshot_declaration` 的 block_id / layer / priority 模式，使用独立的 block_id（如 `”task.active_snapshot”`），避免与其他声明冲突。

**理由：**

- `build_prompt_output()` 每一步都会重新执行，因此这是让 task 更新在”同一 turn 后续步骤”生效的唯一自然挂点。
- 如果只在 submit 时注入，`taskWrite` 只能帮助下一轮，不足以支撑真正的执行记忆。
- 只注入活跃项避免 token 浪费，与 design 的”全部 completed 即清除”语义一致。

**替代方案：**

- 扩展 `RuntimePromptFactsProvider` 去读 session task：不优先采用。当前 provider 不依赖 session-runtime，把 session 级 live truth 拉进 provider 会破坏装配边界。

### Decision 6：Task 读模型挂在 conversation/control surface，不进入 transcript 主体

**选择：**

- 首版 task panel 通过 conversation snapshot/stream 的 control facts 暴露，在 `ConversationControlStateDto` 中新增 `activeTasks` 字段（结构类似 `activePlan`，为 `Option<Vec<TaskItemDto>>`）。
- task 更新不生成新的 transcript 主消息类型，也不占用 `PlanMessage`。
- 当前无 active tasks 时，`activeTasks` 为 `None`，前端隐藏 task 区域。
- **taskWrite 工具调用本身保留为正常 `ToolCallBlock`**，不做抑制（与 plan 工具被 `should_suppress_tool_call_block()` 抑制并转为 `Plan` block 的行为不同）。这样用户可以在 transcript 中看到模型何时调用了 taskWrite。
- 前端 task 区域首版渲染为**对话区顶部的折叠卡片**（位于消息列表上方、TopBar 下方），展示当前 in_progress 任务标题 + pending/completed 计数。不采用独立侧边栏或 TopBar pill，因为 task 信息需要比 badge 更大的展示面积，但又不需要打断对话布局。

**理由：**

- 用户要的是”始终可见的执行记忆面板”，不是”在 transcript 里堆一串 task 更新消息”。
- 当前前端已有 `activePlan` 这类 control state 模式（TopBar pill），task 也适合走持续控制态。
- 保留 taskWrite 作为正常 ToolCallBlock 让 transcript 保持完整的操作审计链。
- 对话区顶部卡片是 TopBar badge（太小）和侧边栏（架构改动大）之间的折中方案。

**替代方案：**

- 把 task 作为普通 ToolCallBlock 展示并依赖用户滚动查看：拒绝。持续可见性差。
- 抑制 taskWrite 工具调用（像 plan 那样转为专属 block）：拒绝。task 更新频率远高于 plan，抑制后 transcript 会丢失重要的执行审计记录。
- 新建独立 HTTP/SSE surface：暂不采用。首版先复用现有 conversation surface 扩展。
- TopBar pill badge：拒绝。面积不足以展示任务列表内容。
- 独立侧边栏：首版范围过大，可作为后续演进。

### Decision 7：`taskWrite` 只在 `code` mode 可用

**选择：**

- `taskWrite` 注册为稳定 builtin tool，通过 `SideEffect::Local`（见 Decision 2）实现 mode 自动过滤：
  - Code mode（`AllTools`）→ 可见。
  - Plan mode（排除 `SideEffect(Local)`）→ 不可见。
  - Review mode（只允许 `SideEffect(None)`）→ 不可见。
- 无需在 mode catalog 中按名字添加白名单或黑名单，复用现有集合代数。

**理由：**

- task 是执行期控制面，不是规划工件。
- 若在 plan mode 允许 task 写入，会让模型用执行 checklist 替代正式 plan 产出。
- 利用 SideEffect 而非按名字排除，保持 mode catalog 的声明式纯净度。

**替代方案：**

- 所有 mode 都暴露 taskWrite：拒绝。会冲淡治理模式边界。
- 使用 `SideEffect::None` 并在 mode catalog 按名字排除：可行但增加了 mode catalog 维护成本，不如直接用 `Local` 自然排除。

### Decision 8：首版不做提醒器、依赖关系和多工具协作

**选择：**

- 首版不实现自动 reminder、task 依赖关系、任务 claim、owner/blocking、跨 session 文件任务库。
- 仅通过 tool prompt metadata 和 task panel 提升模型使用率。

**理由：**

- 这些能力都建立在 task 闭环已经稳定的前提上。
- 当前最有价值的是让模型有”外化的、持久的、动态可见的执行记忆”，不是引入新的协调系统。

**替代方案：**

- 同步实现 Claude Code V1 + V2 的全量能力：拒绝。范围过大。

### Decision 9：`taskWrite` 的 Prompt Guidance

**选择：**

- `taskWrite` 的 `ToolPromptMetadata` 提供详细的使用指导，包含：
  - **何时使用**：任务需要 3+ 步骤、用户给出多个任务、非平凡多步操作、用户明确要求跟踪进度。
  - **何时不用**：单步简单任务、纯对话/信息查询、可在 3 步内完成的琐碎操作。
  - **状态管理规则**：同一时刻最多 1 个 `in_progress`；开始工作前必须先标为 `in_progress`；完成后立即标为 `completed`（不批量）；不再需要的任务直接从列表移除（不保留 completed）。
  - **双形式要求**：每项必须同时提供 `content`（祈使句）和 `activeForm`（进行时），用于 prompt 注入和 UI spinner。
  - **完成标准**：只在真正完成时标 `completed`；测试失败、实现部分、未解决错误时保持 `in_progress`。
- 参照 Claude Code `TodoWriteTool/prompt.ts` 的模式，但精简到 Astrcode 首版的 3 状态模型。

**理由：**

- 没有详细 prompt guidance 的工具通常被模型忽略或误用。Claude Code 的 TodoWrite 之所以有效，很大程度上归功于 180+ 行的精确使用指导。
- 双形式（content + activeForm）让 prompt 注入和 UI 各取所需，不需要系统猜测进行时态。

**替代方案：**

- 只提供简短 description，依赖模型自行推断用法：拒绝。实测表明模型会过度使用（给单步任务建 list）或误用（批量标 completed）。
- 不要求 activeForm：可行但会降低 UI spinner 和 prompt 注入的表达力。

## Risks / Trade-offs

- **[Risk] 全量快照在 task 很多时会增加 token 和工具输入成本**
  → Mitigation：首版通过 20 条上限约束 + tool guidance 鼓励精简列表；prompt 注入只保留 in_progress + pending 摘要。

- **[Risk] 使用 tool result metadata 而非专门事件类型，后续扩展可能遇到 schema 演进压力**
  → Mitigation：从第一版开始定义稳定 `schema` 字段（`"executionTaskSnapshot"`）和版本化 payload；当需求升级到多工具协作时再考虑提升为一等事件。

- **[Risk] task 不进入 transcript 主体，会降低历史可审计性**
  → Mitigation：taskWrite 工具调用本身保留为正常 ToolCallBlock，提供操作审计；durable tool result 完整保留 snapshot 数据。

- **[Risk] owner 维度在当前实现里不够直观**
  → Mitigation：UI 首版只显示当前 session 当前 owner 的 active snapshot，把多 owner 展示留到后续迭代。

- **[Risk] 动态 prompt 注入如果无长度控制，可能和其他 prompt declaration 竞争预算**
  → Mitigation：限制注入只含活跃项（in_progress + pending），已完成项不注入，空列表不生成声明。

- **[Trade-off] 首版不做 `taskRead` 工具**
  → 接受。对模型来说，动态 prompt 注入比显式读取工具更稳定，避免多一个会被误用的工具。

### Decision 10：Application 层 task facts 读取路径

**选择：**

- `session-runtime` 的 `SessionQueries` 新增 `active_task_snapshot(session_id, owner) -> Option<TaskSnapshot>` 查询方法，直接读取 `SessionState.active_tasks` 缓存。
- `application` 层在 `terminal_control_facts()` 中调用该方法（遵循现有 `activePlan` 通过 `session_plan_control_summary()` 读取的模式），将结果映射为 `TerminalControlFacts.active_tasks` 字段。
- `server` 层在 `to_conversation_control_state_dto()` 映射中将 `active_tasks` 转为 `ConversationControlStateDto.activeTasks`。
- 整条链路为 `SessionState → SessionQueries → terminal_control_facts → ConversationControlStateDto`，与 `activePlan` 的三级读取模式完全对齐。

**理由：**

- 与现有 `activePlan` 读取路径保持架构一致性。
- `application` 不直接扫描 tool result 事件，而是通过 `SessionRuntime` 的稳定 query 接口读取。

**替代方案：**

- 在 `SessionControlStateSnapshot` 中直接携带 task 数据：可行但会增加 session-runtime 与 application 的耦合面，不如走 query 方法灵活。

## Migration Plan

1. 在 `crates/core` 引入 task 稳定类型（`ExecutionTaskItem`、`ExecutionTaskStatus`、`TaskSnapshot`）与 metadata schema 结构，但不改现有 durable 事件定义。
2. 在 `crates/adapter-tools` 新增 `taskWrite` 及其输入校验（含 20 条上限）、prompt guidance（`ToolPromptMetadata`）、capability metadata（`SideEffect::Local`）和 tool result metadata 写出。
3. 在 `crates/server/src/bootstrap/capabilities.rs` 注册 `taskWrite`。
4. 在 `crates/session-runtime` 的 `SessionState` 新增 `active_tasks: StdMutex<HashMap<String, TaskSnapshot>>` 字段，在 `translate_store_and_cache()` 中拦截 `taskWrite` 的 `ToolResult` 事件并更新投影。
5. 在 `crates/session-runtime/src/turn/request.rs` 的 `build_prompt_output()` 中新增 `live_task_snapshot_declaration(...)`，参照 `live_direct_child_snapshot_declaration` 模式，只注入 in_progress + pending 项。
6. 在 `crates/session-runtime` 的 `SessionQueries` 新增 `active_task_snapshot()` 查询方法。
7. 在 `crates/application` 的 `terminal_control_facts()` 中调用 `active_task_snapshot()`，将结果映射到 `TerminalControlFacts.active_tasks`。
8. 在 `crates/protocol` 的 `ConversationControlStateDto` 新增 `activeTasks` 字段；在 `crates/server` 的 `to_conversation_control_state_dto()` 映射中填充该字段。
9. 在前端 `ConversationControlState` 类型新增 `activeTasks`，新增 task 卡片组件（对话区顶部折叠卡片），接入 conversation control state 的 hydration 和 delta 更新。

**回滚策略：**

- 如果实现质量不满足预期，可先从 capability surface 中移除 `taskWrite`，前端隐藏 task panel。
- 已存在的 durable tool result metadata 继续留在事件日志中，但旧数据会被新读模型忽略，不影响现有 Plan / transcript 行为。

## Open Questions

- 首版是否需要在 transcript 里保留一条极简 “task updated” system note 作为调试辅助？当前设计默认不需要，这不是 apply-ready 的阻塞项。
- 若后续需要让用户直接编辑任务面板，是否复用 `taskWrite` 作为唯一写入口，还是引入 UI 专属 command？当前设计先保留 `taskWrite` 为唯一写入口。
