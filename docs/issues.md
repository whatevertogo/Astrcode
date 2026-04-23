### [OMX/Workflow] `ultraqa` 残留状态会阻塞 `$ralph` 激活

当前状态：已确认可稳定复现，暂未在本仓库内修复。即使执行 `omx cancel`、`omx state clear --input '{"mode":"ultraqa","all_sessions":true}'`，随后再调用 `omx state write` / `omx_state.state_write(mode="ralph")` 仍可能报 `Cannot write ralph: ultraqa is already active`。本轮通过手动按 Ralph 协议继续执行任务，没有中断实现。

复现步骤：
1. 在 Astrcode 仓库中进入一个曾跑过 `ultraqa` 的会话
2. 执行 `omx cancel`
3. 执行 `omx state clear --input '{"mode":"ultraqa","all_sessions":true}' --json`
4. 再执行 `omx state write --input '{"mode":"ralph","active":true}' --json`

错误日志：
```text
{"error":"Cannot write ralph: ultraqa is already active. Unsupported workflow overlap: ultraqa + ralph. Current state is unchanged. Clear incompatible workflow state yourself via `omx state clear --mode <mode>` or the `omx_state.*` MCP tools, then retry."}
```

### [Plan Mode/E2E] 真实浏览器链路里，plan 请求会长时间停留在 `readFile docs/issues.md` 分页读取，未产出最终计划面板

当前状态：已在本机真实链路复现，暂未修复。普通对话链路可正常创建 session、提交 prompt、返回文本和 Prompt 指标；但当用户在真实页面里发送“进入 plan mode 并为清理 `docs/issues.md` 制定 3 步计划”后，UI 会进入 `plan` 顶栏，随后持续显示 `enterPlanMode` 和多次 `readFile(path=\"docs/issues.md\", offset=...)` 成功，却在 30+ 秒内仍未出现最终 plan block / canonical plan surface。

复现步骤：
1. 启动 `cargo run -p astrcode-server`
2. 启动 `npm --prefix frontend run dev`
3. 打开 `http://127.0.0.1:5173/`
4. 在“新建项目”弹窗中填入 `d:\GitObjectsOwn\Astrcode` 并确认
5. 在真实会话中发送：`请进入 plan mode，为清理 docs/issues.md 制定一个 3 步计划，只输出计划，不要执行。`
6. 观察 30 秒以上的页面状态

错误现象：
```text
- 顶栏 mode 已切为 plan
- tool blocks 依次出现：
  - enterPlanMode 成功
  - readFile(path="docs/issues.md")
  - readFile(path="docs/issues.md", offset=387)
  - readFile(path="docs/issues.md", offset=771)
  - ...
- 页面持续显示 Thinking / 中断按钮
- 未出现最终的 plan surface，也没有完成态 assistant 计划输出
```

### [Core/Eval] `PromptMetricsPayload` 新字段导致 Rust 测试编译失败

当前状态：工作区已修复，`cargo test -p astrcode-core --lib`、`cargo test -p astrcode-eval --lib` 与 `npm run check:push` 已回归通过；待提交后补 commit hash。

复现步骤：
1. 在仓库根目录执行 `npm run check:push`
2. 观察 `cargo test --workspace --exclude astrcode --lib` 编译阶段输出

错误日志：
```text
error[E0063]: missing field `prompt_cache_diagnostics` in initializer of `PromptMetricsPayload`
   --> crates\eval\src\trace\mod.rs:374:30

error[E0063]: missing field `prompt_cache_diagnostics` in initializer of `PromptMetricsPayload`
   --> crates\core\src\event\translate.rs:783:34

error[E0063]: missing field `prompt_cache_diagnostics` in initializer of `PromptMetricsPayload`
   --> crates\core\src\event\types.rs:573:26
```

### [CLI/Protocol] `Conversation*Dto` 新字段导致 CLI 测试编译失败

当前状态：工作区已修复，`cargo test -p astrcode-cli --lib` 与 `npm run check:push` 已回归通过；待提交后补 commit hash。

复现步骤：
1. 在仓库根目录执行 `npm run check:push`
2. 观察 `cargo test --workspace --exclude astrcode --lib` 编译 `astrcode-cli` 阶段输出

错误日志：
```text
error[E0063]: missing field `step_progress` in initializer of `astrcode_client::ConversationStreamEnvelopeDto`
    --> crates\cli\src\app\mod.rs:1718:21

error[E0063]: missing field `step_progress` in initializer of `astrcode_client::ConversationStreamEnvelopeDto`
   --> crates\cli\src\state\conversation.rs:462:13

error[E0063]: missing field `step_progress` in initializer of `astrcode_client::ConversationSnapshotResponseDto`
   --> crates\cli\src\state\mod.rs:455:9
```

### [Conversation/Cache] Prompt Metrics 未投影到前端，Cache Break 指示器不可见

当前状态：工作区已修复，conversation v1 现已透传 `prompt_cache_diagnostics`、`prompt_cache_unchanged_layers`，前端会渲染 Prompt Metrics 与 Cache Break；`npm test`、`npm run typecheck`、`npm run check:push` 已回归通过；待提交后补 commit hash。

复现步骤：
1. 通过 conversation v1 snapshot/stream 返回 `prompt_metrics` block
2. 打开前端聊天视图，观察消息列表

错误现象：
```text
- conversation projection 在 `frontend/src/lib/api/conversation.ts` 的 `prompt_metrics` 分支直接 return
- `frontend/src/components/Chat/MessageList.tsx` 对 `promptMetrics` message 直接 continue
- 结果：后端已生成的缓存诊断不会出现在前端，无法用 Cache 指示器定位 cache break
```

### [Compact] 三种 compact 模式梳理与当前验证结果

当前状态：已完成代码级梳理与回归验证，`incremental` 现在也有直接命名到 `CompactAppliedMeta.mode` 的独立回归；`cargo test -p astrcode-session-runtime --lib` 已整体通过，当前工作区暂未发现确定性失败，后续还需要补真实长对话/端到端压力验证。

模式梳理：
1. `full`
   说明：标准全量 compact；手动 compact 默认走该模式。
   证据：`cargo test -p astrcode-session-runtime build_manual_compact_events_generates_real_summary_event --lib`
2. `incremental`
   说明：基于已有 compact summary 的滚动 compact。
   证据：`cargo test -p astrcode-session-runtime build_compact_result_marks_incremental_mode_when_previous_summary_exists --lib`
3. `retry_salvage`
   说明：compact 请求本身过长时，裁剪最旧 compact unit 后重试的恢复模式；这就是第三种模式。
   证据：`cargo test -p astrcode-session-runtime recovery_result_from_compaction_emits_event_and_appends_file_recovery_messages --lib`

触发方式补充：
- `manual`：立即执行手动 compact
- `deferred`：当前 turn 结束后执行手动 compact
- `auto`：上下文接近阈值时自动触发

验证结果：
- `manual/full`：通过，`finalize_turn_execution_persists_deferred_manual_compact_after_success` 与 `build_manual_compact_events_generates_real_summary_event` 均通过
- `deferred`：通过，`finalize_turn_execution_persists_deferred_manual_compact_after_success` 通过
- `incremental`：通过，`build_compact_result_marks_incremental_mode_when_previous_summary_exists` 直接锁住“已有 compact summary -> Incremental meta”
- `auto/retry_salvage`：通过，`recovery_result_from_compaction_emits_event_and_appends_file_recovery_messages` 与 `compact_applied_event_saturates_large_stats_and_preserves_metadata` 通过

剩余风险：
```text
- 目前证据以单元/组件级回归为主，还没有真实长对话场景下的端到端 cache 命中率数据
- 目前新增的 incremental 证据仍是模块级回归，还没有真实多轮 compact 链路上的端到端压力/恢复验证
```

### [Subagent/Subsession] 子智能体链路、取消语义与前端显示回归验证

当前状态：已完成任务 3 的代码级回归验证，并补上“取消不是 aborted 占位文案”的精确测试；本轮继续补充了真实 conversation snapshot 证据，并修复了一个会导致历史子会话 snapshot 500 的 durable 兼容问题。当前工作区未发现确定性失败，待后续补真实桌面端交互与 provider 超时场景的端到端验证。

验证范围：
1. 子智能体创建、结果回传、子会话落盘与 durable fallback
   - `cargo test -p astrcode-server agent_routes_tests -- --nocapture`
   - `cargo test -p astrcode-application agent::routing::tests:: --lib`
   - `cargo test -p astrcode-application agent::terminal::tests:: --lib`
   - `cargo test -p astrcode-application agent::wake::tests:: --lib`
2. `error.is_cancelled()` / `cancel.is_cancelled()` 相关取消语义回归
   - `cargo test -p astrcode-session-runtime map_kernel_error_restores_llm_interrupted_variant_for_cancelled_messages --lib`
   - `cargo test -p astrcode-application cancelled_child_turn_preserves_interrupted_failure_details --lib`
3. 前端子会话卡片显示与错误信息透传
   - `npm --prefix frontend test -- --run src/lib/subRunView.test.ts src/components/Chat/SubRunBlock.test.tsx`
4. 真实 durable conversation snapshot 扫描
   - 对 `/api/sessions` 返回的全部历史 session 批量请求 `/api/v1/conversation/sessions/{id}/snapshot`
   - 当前结果：`ALL_SNAPSHOTS_OK`
   - 真实 child/subrun 证据：`2026-04-21T22-26-46-782f4530` 与 `2026-04-21T22-28-37-7ae9de46` 的 snapshot 中都能看到 `tool_call` + `child_handoff`，且 `spawn` tool block 携带 `subRunId`、`agentId`、`openSessionId`

当前结论：
```text
- 子智能体创建、resume、向父级回传、durable fallback、wake/requeue 现有回归全部通过
- 取消态现在有显式测试保证：后端保留 Interrupted/technical_message，不回退成 aborted
- 前端 SubRunBlock 现有回归通过，新增测试确认 cancelled 卡片显示“已取消”与精确 technical message
- 真实历史会话的 authoritative snapshot 现在可以稳定读出 child_handoff / child session 事实，不再只依赖单元测试
```

剩余风险：
```text
- 当前证据仍以单元/集成测试为主，尚未复现真实 provider 读超时后的完整桌面端交互链路
- “桌面端前端显示是否正常”目前主要依赖 React/Vitest 视图回归，尚未补 Tauri 侧人工或自动化 UI 冒烟
```

### [已修复] [Storage/Conversation] 历史 `auto_continue_nudge` user origin 导致 conversation snapshot 500

当前状态：工作区已修复。根因是 durable session 文件里的历史 `userMessage.origin = "auto_continue_nudge"` 不再被当前 `UserMessageOrigin` 反序列化接受，导致 `/api/v1/conversation/sessions/{id}/snapshot` 把合法历史文件误判为损坏。当前已为 `ContinuationPrompt` 补上 serde alias，并增加存储层回归测试；待提交后补 commit hash。

复现步骤：
1. 启动本地 server，交换 bootstrap token 获取 API token
2. 请求 `GET /api/v1/conversation/sessions/2026-04-21T22-29-24-661616b0/snapshot`
3. 观察修复前响应

错误日志 / 响应：
```text
HTTP/1.1 500 Internal Server Error
{"code":"internal_error","message":"parse error: failed to parse event at C:\\Users\\18794\\.astrcode\\projects\\D-gitobjectsown-astrcode\\sessions\\2026-04-21T22-29-24-661616b0\\session-2026-04-21T22-29-24-661616b0.jsonl:113 ... The session file may be corrupted."}

113: {"storageSeq":113,...,"type":"userMessage","content":"继续推进当前任务。仅在仍有未完成内容时继续，不要重复已经给出的结论。","timestamp":"2026-04-21T22:33:27.918318400+08:00","origin":"auto_continue_nudge"}
```

修复与验证：
- 兼容修复：`crates/core/src/action.rs` 为 `ContinuationPrompt` 增加 `#[serde(alias = "auto_continue_nudge")]`
- 存储回放回归：`crates/adapter-storage/src/session/iterator.rs`
- `cargo test -p astrcode-core user_message_origin_accepts_legacy_auto_continue_nudge_alias --lib`
- `cargo test -p astrcode-adapter-storage iterator_accepts_legacy_auto_continue_nudge_user_origin --lib`
- `cargo test -p astrcode-adapter-storage --lib`
- 重启本地 `astrcode-server` 后，原先失败的 `GET /api/v1/conversation/sessions/2026-04-21T22-29-24-661616b0/snapshot` 现已返回 `HTTP/1.1 200 OK`
- 继续批量扫描 `/api/sessions` 下所有历史 session snapshot，当前结果：`ALL_SNAPSHOTS_OK`

### [Plan Mode] 进入、退出、状态投影与前端进度显示回归验证

当前状态：已完成任务 4 的代码级与前端组件级回归验证，并补充了 `TopBar` / `TaskPanel` 的计划态显示测试；本轮继续补充了 live server + 浏览器开发态下的真实 mode 切换证据。当前工作区未发现确定性失败。

验证范围：
llm 通过提示词进入plan mode
llm生成你提供的需要执行的计划，并且plan block正确展示在前端
llm通过提示词退出plan mode，或者选择自主退出plan mode
plan mode下的状态跟踪和进度展示
前端桌面端显示正常


当前结论：
```text
- enterPlanMode / exitPlanMode 的 mode transition、review pending、最终呈递流程回归通过
- workflow service 已覆盖 planning <-> executing 的 canonical state 切换与 mode 对齐
- conversation / frontend 已覆盖 activePlan、activeTasks、plan blocks、review-pending card、TopBar 与 TaskPanel 的计划态展示
- live server 上的新 session 可以真实切到 plan mode，浏览器开发态 TopBar 也会同步显示 `plan`
- live provider 路径下的真实 plan 生成也可用，`activePlan`、`plan` block 和 awaiting approval UI 都已拿到实证
```

剩余风险：
```text
- 目前主要是模块级/组件级回归，尚未跑一条真实交互式“进入 plan -> 多次 upsert -> 修改后继续 -> exit”端到端冒烟
- 中途中止/继续执行的证据当前更多来自 workflow/service 级别，而非真实 UI 操作流
```

### [已修复] [Plan Mode/UI] `upsertSessionPlan` 首次失败后，页面会同时显示失败 tool block 与成功 plan block

当前状态：工作区已修复。根因是 conversation projector 只在 `ToolCallStart` 阶段 suppress `upsertSessionPlan` / `exitPlanMode`，但失败 `ToolCallResult` 仍会回退成普通 `tool_call` block；当同一 turn 后续重试成功时，页面就会同时看到失败 tool block 和成功 plan block。当前已统一 suppress 这两类 canonical plan tool 的 start/delta/result fallback，只保留 `plan` surface。

复现步骤：
1. 创建新 session，切到 `plan` mode
2. 提交 `请为清理 docs/issues.md 制定一个 3 步计划，只输出计划，不要执行。`
3. 打开会话页面，观察消息流

修复前现象：
```text
- snapshot 中存在失败的 `tool_call`:
  toolName = upsertSessionPlan
  status = failed
  error = validation error: session plan does not satisfy artifact contract 'canonical-plan' ...

- 但同一 turn 后续又存在成功保存的 `plan` block:
  toolCallId = call_816558e9b6af43e8bc2eb795
  eventKind = saved
  status = awaiting_approval
  title = 清理 docs/issues.md

- Playwright 真实页面同时显示：
  - `upsertSessionPlan 已运行 ... 失败`
  - `计划已更新 / 待确认`
```

修复与验证：
```text
- 代码修复：`crates/session-runtime/src/query/conversation.rs`
  - `ToolCallDelta` 对 suppress tool 直接跳过
  - `ToolCallResult` 在无法投影成 canonical plan block 时，不再为 suppress tool 回退生成普通 `tool_call` block
- 回归测试：`crates/session-runtime/src/query/conversation/tests.rs`
  - 新增 `snapshot_suppresses_failed_upsert_session_plan_retry_noise`
- 自动化验证：
  - `cargo test -p astrcode-session-runtime snapshot_suppresses_failed_upsert_session_plan_retry_noise --lib`
  - `cargo test -p astrcode-session-runtime query::conversation::tests:: --lib`
- 真实 API 复验：
  - 重启本地 `astrcode-server` 后，重新读取 session `2026-04-22T01-24-40-28ee37da` 的 `/api/v1/conversation/sessions/{id}/snapshot`
  - 当前结果只剩 1 个普通 `tool_call`（`readFile`）和 1 个 `plan` block（`call_816558e9b6af43e8bc2eb795`）
  - 原先失败的 `upsertSessionPlan` `call_3f35425c5aaf464ea019f10c` 已不再出现在 authoritative snapshot 中
```

### [Eval] `astrcode-eval` 核心任务集已扩充到 10 条自动化评测

当前状态：已完成任务 5 的核心增量。`eval-tasks/task-set.yaml` 已从 3 条扩到 10 条，覆盖工具调用准确性、compact 上下文保留、plan mode 计划质量/显示、提示词直接响应质量；`cargo test -p astrcode-eval` 与整仓 `npm run check:push` 已回归通过。

新增任务：
1. `prompt-direct-answer`
2. `multi-read-context-summary`
3. `write-plan-checklist`
4. `compact-context-retention`
5. `compact-followup-edit`
6. `plan-review-readiness`
7. `tool-argument-discipline`

验证范围：
1. task set 加载与 fixture 路径解析
   - `cargo test -p astrcode-eval --test core_task_set`
2. mock server 驱动的整套 end-to-end eval 执行
   - `cargo test -p astrcode-eval --test core_end_to_end`
3. eval crate 内部 scorer / runner / diagnosis / trace 单元回归
   - `cargo test -p astrcode-eval --lib`

当前结论：
```text
- task set 当前共 10 条任务，满足“至少 10 个有意义的评测用例”
- core_end_to_end 已验证 10 条任务全部通过，并且 baseline diff 稳定
- 新增任务覆盖了工具精度、计划检查清单、compact 摘要提取、compact 后继续编辑、plan readiness、零工具直接响应等维度
- 本轮继续补跑了整仓 `npm run check:push`，当前增量未引入新的编译、测试或 crate boundary 回归
```

剩余风险：
```text
- 当前 eval 仍以 mock server 驱动为主，尚未引入真实桌面端/真实 provider 的离线回放样本
- compact 与 plan mode 的评测目前更偏“约束保留/产物质量”，还没有更细粒度的多轮行为 judge
```

### [E2E/Browser] 浏览器开发态真实页面冒烟验证

当前状态：已完成一轮浏览器开发态真实交互验证。通过 `cargo run -p astrcode-server` + `frontend npm run dev` 拉起本地链路后，使用 Playwright MCP 打开 `http://127.0.0.1:5173/` 做最小 UI 冒烟，当前未发现阻塞性前端错误。

验证范围：
1. 浏览器桥接与 server bootstrap
   - `GET http://127.0.0.1:5173/__astrcode__/run-info`
   - 返回 payload：`{"token":"...","serverOrigin":"http://127.0.0.1:51726"}`

### [已修复] [Conversation/Stream] 不存在但未超前的 cursor 会静默退回全量 replay

当前状态：工作区已修复。根因是 application 查询层之前只拦截“超前于 head 的 cursor”，没有拦截“格式合法但 transcript 中根本不存在”的 cursor；这类请求会继续落到 `split_records_at_cursor(...)`，由于找不到精确命中而退回 `(Vec::new(), full_records)`，最终表现成整段会话从头重放。

修复与回归：
- 代码修复：
  - `crates/application/src/terminal_queries/snapshot.rs`
  - `crates/application/src/terminal_queries/tests.rs`
  - `conversation_stream_facts(...)` 现在会先读取 transcript，若请求的 cursor 不在 transcript.records 中，也直接返回 `RehydrateRequired(CursorExpired)`，而不是继续走 durable replay
- 自动化回归：
  - `cargo test -p astrcode-application terminal_stream_facts_rehydrates_when_cursor_is_missing_from_transcript --lib`
  - `cargo test -p astrcode-application terminal_stream_facts_returns_replay_for_valid_cursor --lib`

真实链路复验：
1. 修复前，对真实 session `2026-04-22T03-16-44-c5838d32` 请求：
   - `GET /api/v1/conversation/sessions/{id}/stream?cursor=43.1&token=...`
   - 或同一路径配 `Last-Event-ID: 43.1`
2. 两种请求都会从 `id: 3.1` 开始重放整段 draft turn，说明缺失 cursor 被静默退化成全量 replay
3. 修复后，重新启动最新 `astrcode-server` 再请求同一路径：
   - 返回单条 envelope：
     - `id: 53.1`
     - `kind: "rehydrate_required"`
     - `requestedCursor: "43.1"`
     - `latestCursor: "53.1"`
4. 同时做对照验证，使用真实存在的 `cursor=43.0`：
   - 仍会正常从后续事件开始补流，首条为 `id: 44.0`
   - 说明修复没有误伤有效 replay cursor

当前结论：
```text
- 无效但未超前的 cursor 现在不会再静默触发全量 replay
- 前端/客户端收到这类 cursor 时会明确拿到 rehydrateRequired 信号，避免重复渲染整段历史消息
- 真实存在的 cursor replay 行为保持不变
```

### [已修复] [Workflow/Test] `workflow_state_service_round_trips_state_file` 会被并行测试的 home/env 串扰打成假失败

当前状态：工作区已修复。根因不是 workflow state 的持久化逻辑本身不稳定，而是这两个测试之前没有接入统一的 test home/env 隔离；同包里其它并行测试会通过 `ASTRCODE_TEST_HOME` 切换宿主 home，导致 `project_dir()` 偶发把 `workflow/state.json` 写到别的临时目录，进而出现 `os error 3`。

修复与回归：
- 代码修复：
  - `crates/application/src/workflow/state.rs`
  - `workflow_state_service_round_trips_state_file`
  - `load_recovering_downgrades_invalid_json_to_none`
  - 两个测试现在都显式使用 `astrcode_core::test_support::TestEnvGuard::new()`，和其它会改 home/env 的测试共享同一把 env 锁
- 自动化回归：
  - `cargo test -p astrcode-application workflow_state_service_round_trips_state_file --lib`
  - `cargo test -p astrcode-application load_recovering_downgrades_invalid_json_to_none --lib`
  - `cargo test -p astrcode-application --lib`
  - `cargo test -p astrcode-application --lib -- --test-threads=1`
  - 继续补了 2 轮额外的 `cargo test -p astrcode-application --lib`，当前都通过

当前结论：
```text
- 这条失败主要是测试隔离缺口，不是 workflow state 读写语义错误
- 现在 workflow state 测试已经和其它 home/env 敏感测试共享统一隔离机制
- 并行 package 级 `astrcode-application --lib` 本轮已连续多次通过，没有再复现该假失败
```

### [已修复] [Plan Mode/UI] `draft` 计划收到批准语句后，前端不再显示提前泄漏的摘要正文

当前状态：已在 application 侧挡住 `draft + 批准语句` 直接进入执行态，又分别在 session-runtime authoritative snapshot、durable replay frames 与前端 conversation projector 侧补上 turn-local 折叠规则。现在历史泄漏 assistant block 不仅不会被页面渲染，authoritative snapshot 与 SSE catch-up replay 也不会再把这类 `draft-approval` turn 的 `assistant/thinking` 暴露给上层；本轮继续把 plan mode prompt 与 `exitPlanMode` 工具输出都改成“canonical plan surface 是唯一主输出”，真实 raw JSONL 里的内部 review 摘要与冗长计划总结也已被压掉。

自动化修复与回归：
- application 侧：
  - `crates/application/src/session_plan.rs`
  - `crates/application/src/session_use_cases.rs`
  - 当当前 active plan 仍是 `draft`，且用户消息命中批准语义（如 `按这个做，开始吧`）时，注入 `mode-hook:plan:draft-approval-guard`
  - `cargo test -p astrcode-application draft_plan_approval_phrase_stays_in_planning_and_injects_guard_prompt --lib`
  - `cargo test -p astrcode-application approval_persists_executing_phase_before_mode_switch_and_reconciles_later --lib`
- source-level 提示/工具结果收口：
  - `crates/application/src/mode/builtin_prompts/plan_mode.md`
  - `crates/application/src/mode/catalog.rs`
  - `crates/adapter-tools/src/builtin_tools/exit_plan_mode.rs`
  - plan mode 不再要求“exit 后再总结计划”，而是明确 canonical plan surface 已承载主输出；`exitPlanMode` 的 review-pending / success tool result 都会显式告诉模型不要再输出冗长 assistant 正文
  - `cargo test -p astrcode-application builtin_plan_mode_declares_mode_contract_fields --lib`
  - `cargo test -p astrcode-adapter-tools exit_plan_mode_requires_internal_review_before_presenting_plan --lib`
  - `cargo test -p astrcode-adapter-tools exit_plan_mode_returns_review_pending_for_incomplete_plan --lib`
- 前端 projector 侧：
  - `frontend/src/lib/api/conversation.ts`
  - `frontend/src/lib/api/conversation.test.ts`
  - turn-local 条件从“最终 `currentModeId === plan`”收敛为“同一 turn 存在批准语句 + `awaiting_approval/presented` canonical plan”
  - `npm --prefix frontend test -- --run src/lib/api/conversation.test.ts`
  - `npm --prefix frontend run typecheck`
- session-runtime authoritative snapshot 侧：
  - `crates/session-runtime/src/query/conversation/projection_support.rs`
  - `crates/session-runtime/src/query/conversation/tests.rs`
  - snapshot 组装完成后按 turn-local 事实移除 `draft-approval` turn 的 `assistant/thinking`
  - `cargo test -p astrcode-session-runtime snapshot_suppresses_draft_approval_assistant_leakage_even_after_mode_switch --lib`
  - `cargo test -p astrcode-session-runtime --lib`
- session-runtime durable replay 侧：
  - `crates/session-runtime/src/query/conversation/projection_support.rs`
  - `crates/session-runtime/src/query/conversation/tests.rs`
  - `build_conversation_replay_frames(...)` 现在会先求出同一套 hidden block ids，再跳过这些 `assistant/thinking` 的 append/patch/complete deltas
  - `cargo test -p astrcode-session-runtime replay_frames_suppress_draft_approval_assistant_leakage --lib`
  - `cargo test -p astrcode-session-runtime --lib`

真实链路复验：
1. 历史复现 session `2026-04-22T02-16-37-5b23cafe` 的 authoritative snapshot 仍可见同一 turn 内存在：
   - user: `按这个做，开始吧`
   - assistant: `计划已呈递。这是一个纯只读总结任务……`
   - `plan(saved, awaiting_approval)` / `plan(review_pending)` / `plan(presented)`
   - 且 snapshot 末尾 `currentModeId = code`
2. 这证明旧过滤条件失效的根因是：页面按最终全局 mode 判定，而不是按 turn-local 事实判定
3. 先应用前端修复后，重新打开同一 session 的真实页面：
   - TopBar 仍显示 `code`
   - 当前计划仍显示 `PROJECT_ARCHITECTURE.md 核心约束只读总结 (awaiting_approval)`
   - 消息流只保留 canonical plan surface：`计划已更新 / 待确认`、`继续完善中`、`计划已呈递 / 待确认`
   - 那段泄漏的 assistant 摘要正文已不再显示
   - 对应截图：`draft-approval-after-filter-ui.png`
4. 再应用 session-runtime snapshot 修复并重启本地 `astrcode-server` 后，重新请求同一 session 的 authoritative snapshot：
   - 最终结果为 `phase = idle`、`currentModeId = code`、`activePlan.status = awaiting_approval`
   - `turn-1776795598339-867bb066` 现在只剩：
     - `user`
     - `prompt_metrics`
     - `plan(saved, awaiting_approval)`
     - `plan(review_pending)`
     - `plan(presented, awaiting_approval)`
   - 原先那条 `assistant: 计划已呈递。这是一个纯只读总结任务……` 与对应 `thinking` 已不再出现在 authoritative snapshot 中
5. 用新 server 重新加载相同页面后，真实浏览器仍只显示 canonical plan surface，与 authoritative snapshot 一致
   - 对应截图：`draft-approval-authoritative-snapshot-ui.png`
6. 用新 server 对同一 session 发真实 SSE catch-up 请求：
   - `GET /api/v1/conversation/sessions/2026-04-22T02-16-37-5b23cafe/stream?cursor=28.1`
   - 当前返回只包含：
     - `plan(presented, awaiting_approval)`
     - 后续 `prompt_metrics`
   - 结构化检查结果：
     - `containsAssistantPatch = false`
     - `containsThinkingBlock = false`
     - `containsPlanBlock = true`
     - `containsPromptMetrics = true`
   - 这说明真实 SSE catch-up replay 也不再把该 turn 的泄漏 `assistant/thinking` 补发给前端
7. 继续做 source-level 复验：重启最新本地 `astrcode-server` 后，新建 session `2026-04-22T02-53-23-847ea926`，先生成 `draft` 计划，再提交 `按这个做，开始吧`
   - authoritative snapshot 中，该批准 turn 只剩：
     - `user`
     - 多个 `prompt_metrics`
     - `plan(saved, awaiting_approval)`
     - `plan(review_pending)`
     - `plan(presented, awaiting_approval)`
   - 直接读取 raw durable 文件 `C:\Users\18794\.astrcode\projects\D-gitobjectsown-astrcode\sessions\2026-04-22T02-53-23-847ea926\session-2026-04-22T02-53-23-847ea926.jsonl`
   - 新结果：
     - 早先那条“计划通过了最终审查……”的 internal review assistantFinal 已不再落盘
     - 早先那条“计划已呈递，请审阅。总结要点：……”的冗长计划总结 assistantFinal 已不再落盘
     - 最终只剩 1 条极短确认正文：`请确认是否批准执行，或提出修改意见。`
   - 这说明 source-level 虽未完全静默，但已经从“内部 review 摘要 + 冗长计划总结”收敛到“仅保留最小批准提示”
8. 继续把 source-level 收口推进到 turn 级 runtime guard：
   - 代码改动：
     - `crates/core/src/session_plan.rs` 新增 `SESSION_PLAN_DRAFT_APPROVAL_GUARD_MARKER`
     - `crates/application/src/session_plan.rs` 在 draft-approval guard 注入消息里写入稳定 marker
     - `crates/session-runtime/src/turn/runner.rs` 在 `TurnExecutionContext` 上缓存 `draft_plan_approval_guard_active`
     - `crates/session-runtime/src/turn/runner/step/mod.rs` 用 turn 级 guard 统一 suppress 本 turn 的 assistant follow-up，不再依赖最近一次 tool result 顺序
   - 新回归：
     - `cargo test -p astrcode-application draft_plan_approval_phrase_stays_in_planning_and_injects_guard_prompt --lib`
     - `cargo test -p astrcode-session-runtime run_single_step_suppresses_assistant_output_for_draft_approval_guarded_turn --lib`
     - `cargo test -p astrcode-session-runtime run_single_step_suppresses_assistant_output_after_exit_plan_review_pending --lib`
     - `cargo test -p astrcode-session-runtime run_single_step_suppresses_assistant_output_after_exit_plan_presented --lib`
     - `cargo test -p astrcode-session-runtime --lib`
     - `cargo test -p astrcode-application --lib`
   - 最新真实 session 复验：重启本地 `astrcode-server` 后，新建 session `2026-04-22T03-16-44-c5838d32`，先生成 draft，再提交 `按这个做，开始吧`
   - 终态 authoritative snapshot：
     - `phase = idle`
     - `currentModeId = code`
     - 同一 approval turn 仍保留 canonical plan surface：
       - `plan(saved, draft)`
       - `plan(review_pending)`
       - `plan(saved, awaiting_approval)`
       - `plan(review_pending)`
       - `plan(presented, awaiting_approval)`
   - 终态 raw durable 文件 `C:\Users\18794\.astrcode\projects\D-gitobjectsown-astrcode\sessions\2026-04-22T03-16-44-c5838d32\session-2026-04-22T03-16-44-c5838d32.jsonl`
   - 新结果：
     - `approvalTurnId = turn-1776799065824-c33177c1`
     - `assistantFinalCount = 0`
     - approval turn 不再落任何 `assistantFinal`
   - 这说明 source-level 已从“仅保留最小批准提示”进一步收口为“draft-approval turn 完全不落 assistant 正文，只保留 canonical plan surface”
9. 再补 SSE catch-up / replay 证据，确认重连补流不会把 assistant 泄漏重新放出来：
   - 用 `curl.exe -sS -N --max-time 3` 捕获真实 SSE 片段：
     - `GET /api/v1/conversation/sessions/2026-04-22T03-16-44-c5838d32/stream?cursor=43.1&token=...`
   - approval turn 结构化筛查结果：
     - approval turn 命中 replay 事件数：`18`
     - `assistant/thinking` 命中数：`0`
     - 仍可见的 block 类型只有：
       - `user`
       - `prompt_metrics`
       - `plan(saved)`
       - `plan(review_pending)`
       - `plan(presented)`
   - SSE 原始片段中，approval turn 末尾继续只看到：
     - `cursor=48.0` -> `prompt_metrics`
     - `cursor=51.0` -> `plan(presented, awaiting_approval)`
   - 这说明即使通过真实 replay/catch-up 重新补流，新的 draft-approval live turn 也不会再把 `assistant` / `thinking` 泄漏给前端
10. 再补同一 live session 的真实浏览器 UI 证据，确认前端最终展示面与 snapshot / replay 一致：
   - Playwright 打开 `http://127.0.0.1:5173/?sessionId=2026-04-22T03-16-44-c5838d32`
   - TopBar 当前显示：
     - mode = `code`
     - 当前计划 = `清理 docs/issues.md (awaiting_approval)`
   - 对 `body.innerText()` 做 approval turn 局部检查，结果为：
     - `approvalFound = true`
     - `hasThinkingAfterApproval = false`
     - `hasReadFileAfterApproval = false`
     - `hasPlanReviewPendingAfterApproval = true`
     - `hasPlanPresentedAfterApproval = true`
     - `hasApprovalPromptLeak = false`
   - 这说明用户在真实页面中看到的 approval turn 只剩 canonical plan surface，不再出现 `Thinking`、工具执行块或“请确认是否批准执行”这类 assistant 泄漏正文
   - Playwright console：`Errors: 0`
   - 对应截图：`output/playwright/draft-approval-no-assistant-ui.png`
