## Context

当前实现中，`crates/session-runtime/src/turn/tool_cycle.rs` 会直接等待工具返回最终 `ToolExecutionResult`，因此任何长时间运行的工具都会阻塞同一 turn 的推进。现有 `crates/adapter-tools/src/builtin_tools/shell.rs` 虽已支持 stdout / stderr 流式输出，但仍是“一次性、非交互式、完成即返回”的命令工具；`crates/core/src/action.rs`、`crates/core/src/event/domain.rs` 与 `crates/session-runtime/src/query/conversation.rs` 也只覆盖完成态工具结果，没有正式的后台任务通知或终端会话实体。

与此同时，仓库已有两个重要基础：

- `session-runtime` 已经是单会话真相面，适合承接 waitpoint、恢复和 authoritative read model。
- `ToolContext` 已提供 stdout / stderr 增量输出发射能力，说明长任务与终端输出的 live/durable 双写链路已经具备雏形。

本次设计是一个跨 `core / session-runtime / application / adapter-tools / frontend` 的架构演进。它不与 `PROJECT_ARCHITECTURE.md` 的现有原则冲突，反而落实了其中关于事件日志优先、query/command 分离的方向；但实现完成后仍应补充文档，显式记录后台进程监管与终端会话的长期归属。

## Goals / Non-Goals

**Goals:**

- 让长耗时工具不再阻塞当前 turn，而是立即转为后台任务并在完成时发送正式通知。
- 让 `shell` 与持久终端会话共享同一套进程监管基础设施，而不是各自维护私有生命周期。
- 为终端会话建立正式工具合同：启动、写入 stdin、显式读取 stdout/stderr、关闭、失败/丢失反馈。
- 为 conversation authoritative read model 增加后台任务通知与 terminal session 的稳定 block 语义。
- 保证恢复与失败语义明确：Astrcode 进程重启、会话中断、任务取消都必须可观测、可回放、可投影。

**Non-Goals:**

- 本次不实现“跨 Astrcode 进程重启继续附着同一底层 OS 进程”的强恢复能力。
- 本次不做后台任务面板、任务搜索或模型轮询式 `task_get/task_list` 产品面。
- 本次不把所有工具都改造成可后台化；第一阶段只要求 `shell` 和新的终端会话工具接入。
- 本次不新增第二套组合根或让前端直接管理进程真相。

## Decisions

### 1. 不引入 `WaitingTool`，而是采用 Claude Code 风格的后台任务句柄

决策：

- 保持现有 session phase 集合，不新增 `WaitingTool`。
- `shell(background)` 调用立即返回普通 `ToolExecutionResult`，其中 metadata 包含 `backgroundTaskId`、`outputPath`、`notificationMode`、`startedAt` 等纯数据字段。
- 后台执行真相不通过“挂起中的 tool block”表达，而通过独立的后台任务事件与通知块表达。

原因：

- Claude Code 的后台 shell 不是 runtime phase，而是“立即返回 + 后台任务 ID + 完成通知”。
- 这让主 turn 始终保持短生命周期，不必把“等待外部进程”硬塞进 turn 状态机。
- 对 Astrcode 来说，这比引入 suspended turn / waitpoint 更接近你希望的产品体验。

备选方案与否决：

- 方案 A：引入 `WaitingTool` 和 waitpoint。否决，因为这更像 runtime-first 的恢复设计，不像 Claude Code。
- 方案 B：扩展 `InvocationMode`。否决，因为 `unary/streaming` 描述的是传输形态，不是后台任务语义。

### 2. 后台任务与终端会话由 `application` 侧 `ProcessSupervisor` 统一监管

决策：

- 在 `crates/application` 增加 `ProcessSupervisor`，作为全局用例层基础设施。
- 它下辖两个子域：
  - `AsyncTaskRegistry`：一次性后台命令
  - `TerminalSessionRegistry`：持久终端会话
- `server` 只在组合根装配；`session-runtime` 通过稳定端口与 supervisor 通信，不直接持有底层 PTY/进程实现。

原因：

- 进程监管是跨 session 的 live control 基础设施，适合位于 `application`，不应把 PTY/进程实现泄漏到 `session-runtime`。
- `session-runtime` 仍然只负责单会话真相、等待点与恢复，不负责平台进程细节。

备选方案与否决：

- 方案 A：把后台任务 registry 放进 `session-runtime`。否决，因为这会让 `session-runtime` 直接承担跨平台进程实现和全局 live handle 管理。
- 方案 B：把进程真相放到前端或 server handler。否决，因为这违反“Server is the truth”和组合根边界。

### 3. 保留 `shell` 为一次性命令工具，新增显式读写式终端会话工具族

决策：

- 现有 `shell` 保持“一次性命令”语义，只增加 `executionMode: auto|foreground|background`。
- 新增工具族：
  - `terminal_start`
  - `terminal_write`
  - `terminal_read`
  - `terminal_close`
  - `terminal_resize`（可选但建议一期包含）

原因：

- 一次性命令与持久终端会话的生命周期不同。
- 终端会话如果不采用显式 `read/write`，最终还是会回到“等待某段输出何时结束”的隐藏状态机。
- 用 `terminal_write + terminal_read(cursor)` 更接近 Codex 的 session handle，也更符合“不引入 WaitingTool”的约束。

备选方案与否决：

- 方案 A：扩展 `shell`，让同一工具同时承担一次性命令和终端会话。否决，因为 tool call 级语义无法干净表达跨多次交互的终端 session。

### 4. 后台任务与终端会话都使用独立 durable 事件，而不是复用单个 ToolCallDelta

决策：

- 后台 `shell` 的原始 tool call 仍然只记录“任务已启动”这一即时结果。
- 新增独立 durable 事件：
  - `BackgroundTaskStarted`
  - `BackgroundTaskProgressed`
  - `BackgroundTaskCompleted`
  - `BackgroundTaskFailed`
- 终端会话新增独立 durable 事件：
  - `TerminalSessionStarted`
  - `TerminalSessionInputAccepted`
  - `TerminalSessionOutputDelta`
  - `TerminalSessionStateChanged`
  - `TerminalSessionClosed`
- conversation read model 增加 `BackgroundTaskNotificationBlockFacts`、`TerminalSessionBlockFacts` 与对应 patch。

原因：

- `ToolCallDelta` 绑定 `tool_call_id`，适合“一次调用一次结果”。
- 背景任务完成通知和终端会话都跨越单次 tool call，必须拥有自己的主键和事件流。

备选方案与否决：

- 方案 A：所有终端输出都继续挂在 `terminal_start` 那个 tool call 上。否决，因为后续 `terminal_input`、close、lost 等更新无法自然归并。

### 5. 后台任务与终端会话采用“事件日志 + live handle + 通知输入”模型

决策：

- `session-runtime` 不持久化 suspended turn。
- `application` 持有后台任务/终端会话 live handle。
- `core/session-runtime` 持久化的是任务/会话事实和完成通知。
- 若后台任务完成且配置为通知模型继续决策，runtime 将其转成一条内部 queued input 或 system note，而不是恢复旧 turn。
- `ProcessSupervisor` 维护 live handle，不写入 DTO。
- 会话 query 读取后台任务/终端会话事实时，优先基于 durable 事件投影；live handle 仅补充当前可取消、可写入等运行态控制信息。

原因：

- durable 事件是恢复与回放真相。
- live handle 不能直接序列化，也不应污染协议层。
- “继续下一步”由新的输入触发，而不是依赖旧 turn 暂停后复活。

失败与恢复语义：

- 同进程存活时：后台任务完成后发出完成通知；若配置允许，再注入一条内部输入唤醒新 turn。
- Astrcode 进程重启后：
  - 后台任务和终端会话仍可被投影为 running / completed / failed / lost。
  - 若底层进程已丢失，则系统必须追加明确失败或 lost 事件，不能静默消失。

### 6. 前端与 conversation surface 直接消费 authoritative background task / terminal blocks

决策：

- `terminal-chat-read-model` delta/snapshot 增加：
  - `backgroundTask` notification block
  - `terminalSession` block type
  - `terminalSession` output / state patch
- 前端不得继续用 metadata 或相邻 block regroup 猜测后台任务完成与终端会话语义。

原因：

- 工具与终端展示真相必须后端聚合。
- 这与现有 `conversation` 作为 authoritative read surface 的方向一致。

## Risks / Trade-offs

- [风险] 后台任务 live handle 与 durable 状态不一致 → 缓解：durable 事件定义 started/completed/failed/lost 真相，live handle 只补充可操作运行态。
- [风险] 终端 prompt 检测在不同 shell 上不稳定 → 缓解：一期不依赖 prompt 检测，只支持 `wait_until = none|silence|exit`；后续再加 shell-specific prompt heuristic。
- [风险] 新增 PTY 依赖带来跨平台复杂度 → 缓解：将 PTY 封装限制在 adapter/application 交界，核心与协议只看纯数据结构。
- [风险] 工具事件、后台任务事件与终端事件并存导致 read model 更复杂 → 缓解：明确“一次性工具”“后台任务通知”“持久终端会话”是三类 block，不混用主键和状态机。
- [风险] 重启后无法继续控制旧终端进程会让用户预期落差 → 缓解：在 spec 中明确该场景为 lost / failed，而不是承诺透明恢复。

## Migration Plan

1. 在 `core`、`protocol`、`session-runtime` 中增加后台任务通知 / terminal session 纯数据结构。
2. 在 `application` 组装 `ProcessSupervisor`，先接入一次性后台 shell。
3. 改造 `shell` 为可选择 foreground/background，并打通任务输出落盘与完成通知。
4. 新增终端会话工具族与 PTY/pipe 实现，接入 supervisor。
5. 扩展 conversation/query/frontend 渲染 background task 与 terminal session。
6. 同步更新 `PROJECT_ARCHITECTURE.md`，记录新的职责边界。

回滚策略：

- 若后台 shell 路径不稳定，可暂时关闭 `executionMode=background/auto`，保留 foreground shell。
- 若终端会话实现不稳定，可整体关闭 `terminal_*` 工具暴露，不影响一次性 shell 与既有聊天流程。
- 新事件类型保持前后端同批发布；若回滚，先停止暴露新工具，再回滚前端块渲染。

## Open Questions

- 一期是否需要 `terminal_resize` 正式对外暴露，还是先内部保留？
- 后台 shell 的 `auto` 判定阈值应基于超时、命令类型，还是工具显式标记？
- 后台任务完成后默认只通知用户，还是同时生成一条内部输入唤醒模型继续决策？
- `ProcessSupervisor` 的可观测性指标是否需要单独纳入 `runtime-observability-pipeline` 的 spec 更新？
