## 移植参考：旧架构必须保留的机制

> 以下是旧 `session-runtime` / `core` / `server` 中已验证的实现，必须等价迁入新架构。
> codex 在执行迁移任务时，应先读取源文件，理解其实现，再等价搬到目标 crate。

### 必须保留的事件模型（留在 core）

- `core/src/event/types.rs`：`StorageEvent` / `StorageEventPayload` / `StoredEvent` / `CompactTrigger` / `CompactMode` / `CompactAppliedMeta` / `TurnTerminalKind` / `PromptMetricsPayload`。
  - 20+ 种事件变体原样保留。
  - `storage_seq` 单调递增由 session writer 独占分配。
  - 校验规则（SessionStart 禁止 turn_id/agent，SubRun 要求 child_session_id）原样保留。
- `core/src/event/` 中 `EventTranslator`、`AgentEvent`、`SessionEventRecord` 等翻译层原样保留。

### 必须保留的持久化合同（留在 core 或迁入 host-session）

- `core/src/store.rs`：`EventLogWriter`、`EventStore`、`SessionManager`、`SessionTurnLease`、`SessionTurnAcquireResult`、`SessionTurnBusy`、`StoreError`。
  - 这些 trait 是跨 crate 共享的稳定合同，继续留在 core。
  - `FileSystemSessionRepository`（adapter-storage）实现不变。

### 必须迁入 host-session 的机制

- **ProjectionRegistry**（`session-runtime/src/state/projection_registry.rs`）：
  - 增量投影：对每条 StoredEvent 做 `apply()` 更新 AgentState、turn 投影、child nodes、active tasks、input queue、mode state。
  - `from_recovery()`：从 checkpoint + tail events 重建完整投影。
  - `snapshot_projected_state()` → `AgentState`。
- **SessionState**（`session-runtime/src/state/mod.rs`）：
  - 组合 projection + writer + 双通道广播。
  - `append_and_broadcast(event, translator)`：append → apply → translate → broadcast。这是事件写入的唯一生产路径。
  - `translate_store_and_cache(stored, translator)`：validate → apply projection → translate → cache records。
  - 双通道广播：`broadcaster: Sender<SessionEventRecord>`（durable）和 `live_broadcaster: Sender<AgentEvent>`（live）。
  - `from_recovery(writer, checkpoint, tail_events)` 恢复流原样保留。
- **SessionWriter**（`session-runtime/src/state/writer.rs`）：
  - 异步 `EventStore` 包装 + 同步 `EventLogWriter` 兼容层。
  - `append()` 异步写入，测试态走 `spawn_blocking` 桥接同步 writer。
- **恢复模型**：
  - `SessionRecoveryCheckpoint`（core）原样保留。
  - 恢复流：open_event_log → 读取 checkpoint → replay tail events → SessionState::from_recovery。
  - checkpoint 包含 `childNodes`、`activeTasks`、`inputQueueProjectionIndex` 等投影快照。
- **事件广播常量**：`SESSION_BROADCAST_CAPACITY = 2048`、`SESSION_LIVE_BROADCAST_CAPACITY = 2048`。

### 必须迁入 agent-runtime 的机制

- **run_turn**（`session-runtime/src/turn/runner.rs`）：
  - 主循环：`loop { run_single_step() → StepOutcome::Continue/Completed/Error }`。
  - 每步后 `flush_pending_events()` 批量写入事件。
  - 取消时写入 `TurnDone(Cancelled)` 后退出。
  - 返回 `TurnRunResult`。
- **TurnRunRequest**（`session-runtime/src/turn/request.rs`）：
  - 输入结构：session_id, working_dir, turn_id, messages, event_store, session_state, cancel, agent context, prompt_declarations, capability_router, prompt_governance。
  - 拆分为：agent-runtime 只接收最小执行面（`TurnInput`），host-session 负责装配其余部分。
- **TurnExecutionContext / TurnExecutionResources**（`session-runtime/src/turn/runner.rs`）：
  - execution 上下文和资源持有。
  - step 执行、journal 管理、lifecycle 转换。
- **流式输出**（`session-runtime/src/turn/` 中 stream 相关）：
  - provider streaming → AssistantDelta 事件 → live broadcast。
  - tool streaming → ToolCallDelta 事件。
- **取消传播**（`session-runtime/src/turn/` 中 cancel 相关）：
  - cancel token → provider abort → pending tool abort → TurnDone(Cancelled)。

### server 组合根保留的设计

- `ServerBootstrapOptions`：可覆盖选项（home_dir, working_dir, plugin_search_paths, enable_profile_watch, watch_service_override）。
- `ServerBootstrapPaths`：从 options 解析路径。
- profile watch runtime 和 MCP warmup 后台任务模式。
- config 覆盖层（用户级 → 项目级）。

---

## 0. 删除验收清单

> 这一组不是“第一步就执行删除”，而是本次 change 的最终验收条件。
> 实际执行顺序应先完成 `1` 到 `7.5` 的迁移、切换与清理，再回到这里做总收口。
> 仓库允许保留旧源码目录作为迁移归档源，但它们不得继续参与活跃 workspace、正式依赖图或对外 surface。

- [x] 0.1 移除整个旧边界的正式 workspace 成员与正式依赖：`crates/application/**`、`crates/kernel/**`、旧 `crates/session-runtime/**`，以及被 `plugin-host` 取代后的旧 `crates/plugin/**` 只允许作为迁移归档源保留；以活跃 workspace / 正式 crate 依赖图中不再出现这些旧边界为验证。
- [x] 0.2 收缩 `core` 的正式公开面，只保留跨 owner 共享语义；`projection/**`、`session_plan.rs`、`composer.rs`、`plugin/registry.rs`、`session_catalog.rs`、旧 `PluginManifest`、旧 `HookInput` / `HookOutcome`、`SubRunHandle`、`InputQueueProjection` 等 owner-only 能力不再作为 `core` 顶层默认导出，新调用方统一通过 owner bridge（主要是 `host-session` / `plugin-host`）消费；`ModeId`、durable event DTO、runtime config、observability wire metrics、`store.rs` 等跨 owner 稳定合同按既定例外保留。
- [x] 0.3 移除 `application` 作为正式编排入口：`src/agent/**`、`src/execution/root.rs`、`src/execution/subagent.rs`、`src/governance_surface/**`、`src/mode/**`、`src/workflow/**`、`src/mcp/mod.rs`、`src/composer/mod.rs`、`src/ports/**` 等旧实现点只允许作为迁移归档源保留，不能再作为活跃实现入口或正式依赖。
- [x] 0.4 移除多 agent 协作的旧跨层真相承载点：旧 `kernel` / `session-runtime` / `application` 中分散的 subrun 持久化、结果回传、取消传播和 input queue 真相不再参与正式路径；以 collaboration durable truth 只剩 `host-session` owner 为验证。
- [x] 0.5 移除 server 组合根里的旧并列事实源与旁路装配语义：`runtime.rs`、`providers.rs`、`plugins.rs`、`mcp.rs`、`governance.rs`、`capabilities.rs`、`watch.rs`、`deps.rs`、`runtime_coordinator.rs` 等剩余文件只允许作为 server-owned bridge / facility 存在，不得再回退到 application/kernel/session-runtime 式手工并列装配；未参与编译的旧旁路文件（如 `prompt_facts.rs`）必须删除。
- [x] 0.6 项目无向后兼容：活跃 workspace、正式依赖图和对外 surface 不再承诺旧 crate / 旧装配路径 / 旧 facade 的兼容性；必要的 compat 仅允许留在私有 bridge/helper 中，不能重新升级为正式入口。

## 1. 架构定版与 crate 骨架

- [x] 1.1 更新 `PROJECT_ARCHITECTURE.md`，把 `agent-runtime`、`host-session`、`plugin-host` 写成新的正式边界，并删除 `application`、`kernel`、monolith `session-runtime` 的长期权威定义。
- [x] 1.2 更新 `node scripts/check-crate-boundaries.mjs` 与 workspace 依赖规则，禁止新边界回头依赖旧 `application` / `kernel` / monolith `session-runtime`。
- [x] 1.3 在 `crates/` 下新建 `agent-runtime`、`host-session`、`plugin-host` crate，并更新根 `Cargo.toml`、workspace 文档与最小公开入口，确保空骨架能编译通过。

## 2. 收缩 `core`

- [x] 2.1 重构 `crates/core` 导出面，只保留 `ids`、消息模型、`CapabilitySpec`、最小 prompt/hook 共享语义；把 `PluginDescriptor` / `PluginActiveSnapshot` / descriptor 家族迁入 `plugin-host`，把 `AgentRuntimeExecutionSurface` 迁入 `agent-runtime`，把 `HostSessionSnapshot` 与恢复/投影模型迁入 `host-session`。
- [x] 2.2 拆散 `crates/core/src/ports.rs`：
  - `EventStore`、`EventLogWriter`、`SessionManager`、`SessionTurnLease`、`SessionTurnAcquireResult`、`SessionTurnBusy` → 留在 core（跨 crate 共享合同）。
  - `PromptDeclaration`、`PromptGovernanceContext` → 留在 core（被 prompt 组装和 hooks 共用）。
  - `PromptProvider`、`PromptFactsProvider` → 迁入 host-session（owner 专属）。
  - `ResourceProvider` → 迁入 plugin-host（owner 专属）。
  - `LlmProvider` → 迁入 agent-runtime（owner 专属）或独立 provider 合同模块。
  - 验证：`core::ports` 不再作为 mega 入口。
- [x] 2.3 从 `crates/core/src/agent/*` 迁出协作模型：
  - `SubRunHandle`、`InputQueueProjection` → host-session owner bridge。
  - `CollaborationExecutor`、`SubAgentExecutor` 相关合同 → host-session owner bridge。
  - `AgentEventContext`、`ChildSessionNotification`、`SubRunResult` → 留在 core（跨 crate 共享）。
  - `ChildAgentRef`、`ChildSessionNode`、`ChildSessionLineageKind` 暂留 core，作为 `ChildSessionNotification` / `StorageEventPayload` 的嵌入式 durable event DTO；后续只有在事件 DTO 拆分出稳定 wire schema 后再迁出。
  - 验证：新调用方通过 host-session owner bridge 消费执行/read-model 类型和协作执行合同；`core` 顶层导出面不再暴露 `SubRunHandle`、`InputQueueProjection`、`CollaborationExecutor`、`SubAgentExecutor`。
- [x] 2.4 迁出或删除 `crates/core/src/projection/`、`workflow.rs`、`plugin/registry.rs`、`session_catalog.rs` 等可安全迁移的 owner 专属模块，并记录 `mode` / config / observability 的共享合同例外：
  - `projection/` → host-session（由 ProjectionRegistry 消费）。
  - `workflow.rs` → host-session。
  - `mode/` 中 `ModeId`、durable mode-change event DTO、`ToolContext` 所需的 bound tool contract snapshot 暂留 core；治理 DSL / builtin mode owner 逻辑在 6.3 迁往 plugin-host，避免当前反向依赖。
  - `plugin/registry.rs` → plugin-host（已有等价实现）。
  - `session_catalog.rs` → host-session。
  - `session_plan.rs` → host-session。
  - `config.rs` 中 runtime config / resolved config 暂留 core 作为 session-runtime、server、adapter-storage 的共享合同；配置持久化和 owner-only 解析逻辑归 application/server，后续协议拆分后再继续收缩。
  - `observability.rs` 中 wire metrics snapshot 暂留 core 供 application 与 protocol 共享；collector / governance summary 已归 application，后续协议 DTO 独立后再迁出。
  - `store.rs` → 留在 core（跨 crate 共享合同）。
  - `composer.rs` → host-session。
  - 验证：`core` 导出面显著收缩；`cargo check --workspace` 通过；`core` 不再导出 `projection`、`workflow`、`session_plan`、`session_catalog`、`composer`、`PluginRegistry`。
- [x] 2.5 删除旧窄版 `PluginManifest`（`core/src/plugin/manifest.rs`）、旧 `HookInput` / `HookOutcome`（`core/src/hook.rs`）相关公开暴露，确保新实现只围绕 descriptor / envelope / effect 模型展开。

## 3. 建立最小 `agent-runtime`

- [x] 3.1 在 `crates/agent-runtime` 中建立骨架模块和 `execute_turn` 入口。
- [x] 3.2 从旧 `session-runtime/src/turn/runner.rs` 迁出 turn 主循环：
  - 源：`run_turn(kernel, TurnRunRequest)` → 目标：`AgentRuntime::execute_turn(TurnInput) → TurnOutput`。
  - 迁出 `run_single_step()` 和 `StepOutcome::Continue/Completed/Error`。
  - 迁出 `TurnExecutionContext` 和 `TurnExecutionResources`。
  - 迁出 `flush_pending_events()` 的事件批处理逻辑（改为通过回调发出，不直接写 EventStore）。
  - 迁出 `TurnStopCause` 和 turn terminal 判断。
  - 关键约束：agent-runtime 不持有 EventStore、不持有 SessionState、不持有 PluginRegistry。
    所有有状态依赖通过 TurnInput 传入或通过 `emit_event` 回调发出。
  - 验证：`execute_turn` 可驱动空流程和基本 turn 生命周期。
- [x] 3.3 迁出 provider 请求和 continuation cycle：
  - 源：`session-runtime/src/turn/runner.rs` 中 provider 调用路径。
  - 迁出 provider stream → `AssistantDelta` / `ThinkingDelta` / `AssistantFinal` 事件的产出。
  - 迁出 continuation 判断（stop / tool_use / continue）。
  - agent-runtime 通过 `TurnInput.provider_ref` 路由到具体 LLM 实现，不硬编码 OpenAI。
  - 验证：provider turn 集成测试通过。
- [x] 3.4 迁出 tool dispatch 和 tool-result 回环：
  - 源：`session-runtime/src/turn/` 中 tool call 执行和结果处理。
  - 迁出 tool call → `ToolCall` / `ToolCallDelta` / `ToolResult` 事件的产出。
  - 迁出 tool result → continuation 判断。
  - agent-runtime 通过 `TurnInput.tool_specs` 匹配工具，通过 plugin-host dispatch 调用。
  - 验证：tool call / tool result 测试通过。
- [x] 3.5 接入 hooks 调度链：
  - 迁出 hook dispatch 逻辑，覆盖 `turn_start` / `context` / `before_agent_start` / `before_provider_request` / `tool_call` / `tool_result` / `turn_end`。
  - hooks 通过 `TurnInput.hook_snapshot_id` 查找，不直接持有 hook registry。
  - 实现 dispatch mode：顺序 / 可取消 / 可拦截 / 可修改 / 管道式 / 短路式。
  - 验证：hooks 顺序、阻断、修改语义测试通过。
- [x] 3.6 迁出取消和流式传播控制：
  - 源：`session-runtime/src/turn/` 中 cancel token 和 streaming。
  - 迁出 cancel → provider abort → pending tool abort → `TurnDone(Cancelled)` 传播。
  - 迁出 streaming delta 通过 `emit_event` 回调发出。
  - 验证：取消与流式恢复测试通过。
- [x] 3.7 验证 child-session 执行合同边界：
  - agent-runtime 只执行 child turn，不持有 `SubRunHandle`、不感知 input queue、不感知父子关系。
  - 协作上下文通过 `TurnInput.agent` 中的 `AgentEventContext` 传入。
  - 验证：runtime 公共 API 审查无协作状态泄漏。

## 4. 建立 `host-session`

- [x] 4.1 迁入事件日志基础设施：
  - 从 `session-runtime/src/state/writer.rs` 迁入 `SessionWriter`（异步 EventStore 包装 + 同步 EventLogWriter 兼容层）。
  - 从 `session-runtime/src/state/projection_registry.rs` 迁入 `ProjectionRegistry`（增量投影 + from_recovery）。
  - 从 `session-runtime/src/state/mod.rs` 迁入 `SessionState`（projection + writer + 双通道广播）。
  - 保留 `append_and_broadcast(event, translator)` 作为事件写入唯一生产路径。
  - 保留双通道广播：`broadcaster: Sender<SessionEventRecord>`（durable）和 `live_broadcaster: Sender<AgentEvent>`（live）。
  - 保留广播容量常量 `SESSION_BROADCAST_CAPACITY = 2048`。
  - 迁入 `translate_store_and_cache(stored, translator)` 流程。
  - 从 `session-runtime/src/state/execution.rs` 迁入 `append_and_broadcast` 自由函数和 `checkpoint_if_compacted`。
  - 验证：恢复、投影和广播测试通过。
- [x] 4.2 迁入恢复流和 session catalog：
  - 从 `session-runtime/src/state/mod.rs` 迁入 `SessionState::from_recovery(writer, checkpoint, tail_events)`。
  - 保留恢复流：open_event_log → 读取 checkpoint → replay tail events → from_recovery。
  - 从 `session-runtime/src/lib.rs` 迁入 `LoadedSession` 和 session DashMap。
  - 保留 `SessionManager::try_acquire_turn` → `SessionTurnAcquireResult::Acquired/Busy` → RAII lease。
  - 从 `session-runtime/src/catalog/` 迁入 session catalog（list/create/delete）。
  - 验证：session 恢复、创建和列表测试通过。
- [x] 4.3 迁入 branch/fork/compact：
  - 从 `session-runtime/src/turn/branch.rs` 迁入 branch 逻辑。
  - 从 `session-runtime/src/turn/fork.rs` 迁入 fork 逻辑。
  - 从 `session-runtime/src/turn/compaction_cycle.rs` 和 `manual_compact.rs` 迁入 compaction。
  - 从 `session-runtime/src/turn/watcher.rs` 迁入 compaction watcher。
  - compaction 产出 `CompactApplied` 事件，通过 `append_and_broadcast` 持久化。
  - 验证：branch/fork/compact 测试通过。
- [x] 4.4 迁入多 agent 协作 durable truth：
  - `host-session` 已有 `SubRunHandle`、`InputQueueProjection`、`HostSession`（spawn_child/send_to_child/send_to_parent/observe_subtree/terminate_subtree）。
  - 从旧实现迁入持久化集成：
    - `session-runtime/src/turn/finalize.rs` 中 subrun finished 持久化 → host-session 的事件写入。
    - `session-runtime/src/turn/interrupt.rs` 中 cancel subruns → host-session 的 terminate_subtree + 事件写入。
    - `session-runtime/src/state/child_sessions.rs` 中 child node 追踪 → host-session 的 lineage 管理。
    - `session-runtime/src/state/input_queue.rs` 中 input queue 投影 → host-session 的 InputQueueProjection 持久化。
  - 落地"一个 session 即一个 agent"的协作模型：父 turn 发起 child session → host 记录 durable linkage → host 调用 agent-runtime 执行 child turn。
  - 验证：child-session 恢复、取消和结果回传测试通过。
- [x] 4.5 迁入 query/read model：
  - 从 `session-runtime/src/query/` 迁入 query 服务（conversation snapshot, subrun query, conversation stream replay）。
  - 从 `session-runtime/src/observe.rs` 迁入 observe 机制（SessionObserveSnapshot, wait_for_turn_terminal_snapshot）。
  - query 直接读 ProjectionRegistry，不额外持久化。
  - 验证：query 和 observe 测试通过。
- [x] 4.6 为 `host-session` 提供正式协作用例入口（已实现：spawn_child, send_to_child, send_to_parent, observe_subtree, terminate_subtree）。

## 5. 建立 `plugin-host` 与统一 surface

- [x] 5.1 实现 descriptor 校验、registry、candidate snapshot、active snapshot、commit/rollback 和 revision 管理。
- [x] 5.2 实现 builtin plugin backend 与 external plugin backend 共用同一 descriptor/snapshot 语义。
- [x] 5.3 统一资源发现：
  - 将 skills、prompts、themes、commands、其他资源入口统一纳入 `resources_discover` 与 `PluginDescriptor` 聚合流程。
  - 从 `server/src/bootstrap/composer_skills.rs` 迁入 skill catalog 构建。
  - 从 `server/src/bootstrap/prompt_facts.rs` 迁入 prompt facts 组装。
  - 移除平行发现路径。
  - 验证：统一资源目录测试和冲突校验测试通过。
- [x] 5.4 统一 provider contribution：
  - 将 provider 纳入 `plugin-host` 的统一 registry / active snapshot。
  - 从 `server/src/bootstrap/providers.rs` 迁入 provider 选择和实例化。
  - 去除 `provider_kind == openai` 硬编码，改为通过 `ProviderDescriptor` 注册。
  - 验证：provider registry 集成测试通过。
- [x] 5.5 将 builtin tools 迁为 builtin plugin：
  - 从 `server/src/bootstrap/capabilities.rs` 中 `build_core_tool_invokers` + `build_agent_tool_invokers` → 改为 builtin `PluginDescriptor`（tools 字段填充 builtin tools）。
  - 从 `server/src/bootstrap/mcp.rs` 中 MCP invokers → 改为 builtin 或 external `PluginDescriptor`（tools 字段填充 MCP tools）。
  - 所有 invoker 注册走 `plugin_host.reload_with_builtin_and_loader()`。
  - 验证：builtin tools 通过 plugin-host active snapshot 可查可执行。
- [x] 5.6 将协作入口收敛为 builtin plugin tools：
  - `spawn_agent`、`send_to_child`、`send_to_parent`、`observe_subtree`、`terminate_subtree` → builtin `PluginDescriptor` 的 tools 字段。
  - 这些 surface 只调用 `host-session` use-case，不持有 collaboration durable truth。
  - 验证：协作入口集成测试通过。
- [x] 5.7 实现 hooks 统一扩展总线：
  - 定义 `HookDescriptor`（已在 descriptor.rs 中）。
  - 实现事件分发语义：顺序 / 可取消 / 可拦截 / 可修改 / 管道式 / 短路式。
  - 覆盖事件面：input, context, before_agent_start, before_provider_request, tool_call, tool_result, turn_start, turn_end, session_before_compact, resources_discover, model_select。
  - governance prompt hooks 继续通过 `PromptDeclaration` / `PromptGovernanceContext` 进入 prompt 组装，不新增平行 prompt 渲染系统。
  - 验证：hooks 顺序、阻断、修改语义测试通过。

## 6. 切换组合根与删除旧边界

- [x] 6.1 重写 server 组合根装配路径：
  - 旧路径：手工拼接 core_tool_invokers + agent_tool_invokers + mcp_invokers + plugin_invokers + capability_sync + kernel + application + session_runtime。
  - 当前桥接路径：
    ```
    build_server_plugin_contribution_descriptors(...)
      → reload_server_plugin_host_snapshot(...)
      → PluginActiveSnapshot + ResourceCatalog + ProviderContributionCatalog
    ```
  - 迁移期允许旧 `application` / `kernel` / `session-runtime` 继续承载现有 HTTP/API 调用面，但 server bootstrap 中 plugin/provider/resource 的生效事实必须先收敛为同一组 `PluginDescriptor[]` 产物。
  - `host_session = HostSession::new(...)`、`agent_runtime = AgentRuntime::new()` 和旧 crate 正式依赖删除保留到 6.5 / 0.* 验收清单执行，避免本任务同时跨越 API 调用方切换和旧边界删除。
  - 保留 `ServerBootstrapOptions` 和 `ServerBootstrapPaths`。
  - 保留 profile watch runtime 和 MCP warmup 后台任务模式。
  - 验证：server 编译通过和启动冒烟验证。
- [x] 6.2 重写 provider 装配：
  - 旧：`server/src/bootstrap/providers.rs` 中 `ConfigBackedLlmProvider` 对 `provider_kind != openai` 报错。
  - 新：通过 plugin-host 的 `ProviderDescriptor` 注册 provider，server 不再硬编码 provider kind。
  - 验证：新增 provider 不再要求改组合根。
- [x] 6.3 迁移 governance / mode / workflow 到 plugin/host 层：
  - 当前桥接路径：`ModeCatalog` / `builtin_mode_catalog()` 的生效输入先迁为 plugin-host descriptor 的 `modes` 贡献，server 从 `PluginActiveSnapshot` 派生 builtin/plugin mode catalog。
  - `GovernanceSurfaceAssembler` / `AppGovernance` 与 `WorkflowOrchestrator` 迁移期继续作为旧 API 调用面的消费者存在，但不得再作为 mode 生效事实源；完整 owner 删除放到 6.5 / 0.*。
  - 验证：builtin/plugin modes 通过 plugin-host active snapshot 进入 mode catalog，server bootstrap 不再直接用旁路 `plugin_modes` 替换 catalog。
- [x] 6.4 更新 sdk / protocol / adapter-* 调用方：
  - 统一改用新 DTO、hooks catalog 和 plugin descriptor。
  - 不再暴露旧 `application` / `kernel` / `session-runtime` 内部类型。
  - 验证：全 workspace 编译通过和类型错误清零。
### 6.5 分阶段删除旧边界

> `6.5` 不再作为一次性删除任务执行。旧 `application` / `kernel` / `session-runtime` / `plugin` 仍被 server/API 调用面编译依赖，必须先按下面顺序切换调用方，再执行 `0.*` 删除验收。

- [x] 6.5.1a 切换 config / model API 面：
  - 将 `routes/config.rs`、`routes/model.rs`、provider/profile resolution 和 config selection 从 `App::config()` 迁到 server-owned config/profile service 或新 owner service。
  - `ApplicationError` 映射同步替换为 server/core error 映射。
  - 验证：config/model 路由不再通过 `state.app` 访问配置。
- [x] 6.5.1b-1 切换 session catalog CRUD / fork / catalog stream API 面：
  - 将 `routes/sessions/query.rs`、`routes/sessions/mutation.rs`、`routes/sessions/stream.rs` 的 list/create/delete/delete_project/fork/catalog stream 调用迁到 server-owned `host-session::SessionCatalog`。
  - fork 迁移期允许保留 server-side plan artifact copy bridge，但 catalog durable truth 必须由 `host-session` 写入事件日志。
  - 验证：session catalog CRUD/fork/catalog stream 路由不再通过 `state.app` 访问 session catalog 用例。
- [x] 6.5.1b-2a 建立 `host-session` turn mutation 合同：
  - 在 `host-session` 定义 submit/compact/interrupt 的 owner 输入输出类型与 facade，覆盖 `PromptAcceptedSummary`、`CompactSessionSummary`、interrupt accepted 语义。
  - 合同必须显式区分 `host-session` 拥有的 durable turn mutation 与迁移期仍由 server/application bridge 提供的 governance/workflow/skill-invocation 准备。
  - 验证：新合同不依赖 `application`、旧 `session-runtime` 或 `kernel`。
- [x] 6.5.1b-2b 迁移 submit acceptance / branch-on-busy 归属：
  - 将 turn id 生成、busy lease 获取、branch-on-busy 目标解析和 `ExecutionAccepted` 等价摘要迁到 `host-session::SessionCatalog` / turn mutation facade。
  - `application` 可短期只作为 prompt/governance/workflow 准备 bridge，不再拥有 submit target 决策。
  - 验证：submit acceptance 单测覆盖空输入、busy branch、reject-on-busy 和 accepted response shape。
- [x] 6.5.1b-2c 接通 `agent-runtime` 执行事件持久化：
  - 将 `agent-runtime::RuntimeTurnEvent` 映射到 `host-session` durable event append / projection / broadcast / checkpoint 路径。
  - 保留必要 provider/tool/hook dispatcher bridge，但 turn loop 执行结果不得再由旧 `session-runtime` finalizer 作为唯一持久化入口。
  - 验证：最小 turn 执行能通过 `host-session` 持久化 user/assistant/terminal 事件，并能由 read model 恢复。
- [x] 6.5.1b-2d 迁移 compact / interrupt owner 行为：
  - 将 manual compact 的立即执行/延迟登记、interrupt cancel token、terminal cancelled event、pending compact flush 迁到 `host-session` turn mutation facade。
  - 子运行取消传播可通过临时 executor bridge 调用现有协作入口，但 durable 状态必须由 `host-session` 记录。
  - 验证：compact/interrupt contract tests 不再依赖旧 `session-runtime` 方法。
- [x] 6.5.1b-2e 切换 server submit/compact/interrupt 路由：
  - 将 `routes/sessions/mutation.rs` 的 submit_prompt/compact_session/interrupt_session 从 `state.app` 切换到 server-owned `host-session` turn mutation facade。
  - server 可保留协议 DTO mapper，但不得调用 `application::App` turn/session mutation use-case。
  - 验证：submit/compact/interrupt 路由不再通过 `state.app` 访问 turn 用例，`cargo check -p astrcode-server` 与 session contract tests 通过。
- [x] 6.5.1b-3 切换 session mode API 面：
  - 将 list_modes/get_session_mode/switch_mode 调用迁到 `plugin-host` mode catalog 与 `host-session` mode state owner。
  - 验证：mode 相关 session 路由不再通过 `state.app` 访问 mode 用例。
- [x] 6.5.1c 切换 conversation / terminal read-model API 面：
  - 将 `routes/conversation.rs`、`terminal_projection.rs`、terminal resume/snapshot/stream facts 从 `application::terminal_queries` 迁到 `host-session` query/read-model 与 server projection adapter。
  - 验证：conversation/terminal 路由不再引用 `astrcode_application::terminal*`。
- [x] 6.5.1d 切换 composer / resource discovery API 面：
  - 将 composer options、skills、commands、prompt/theme/resource discovery 从 `application::composer` 迁到 `plugin-host::ResourceCatalog` / descriptor-derived catalog。
  - 验证：composer 路由和 mapper 不再引用 `ComposerOption*` 的 application 类型。
- [x] 6.5.1e 切换 agent collaboration API 面：
  - 将 agent status、root execute、close/observe 和 builtin collaboration tools 从 `AgentOrchestrationService` 迁到 `host-session` collaboration use-case 与 `plugin-host` collaboration surface。
  - 验证：agent routes 和 builtin collaboration invokers 不再依赖 `application::agent`。
- [x] 6.5.1f 移除 server 的 `application::App` 状态入口：
  - 分阶段完成：先移除生产路由/状态对 `App` facade 的依赖，再清理 bootstrap/watch/测试辅助中的 `App` 过渡桥，最后删除 `server` 对 `astrcode-application` 的正式依赖。
  - 只有全部子任务完成后，才能视为 `6.5.1f` 完成。
- [x] 6.5.1f-1 移除生产态 `App` facade 路由入口：
  - `ServerRuntime` / `AppState` 在生产路径上显式持有 agent/config/session/profile/MCP/resource/mode/governance 等 owner service，agent/composer/MCP 路由不再通过 `App` facade。
  - 允许迁移期保留 `cfg(test)` 的 `App` shim，仅供未迁完的 server 测试辅助复用。
  - 验证：生产构建下 `server` 路由代码不再引用 `state.app` / `runtime.app`，`cargo check -p astrcode-server` 通过。
- [x] 6.5.1f-2 清理 bootstrap/watch/测试辅助里的 `App` 过渡桥：
  - profile watch 改为直接消费 `SessionCatalog + ProfileResolutionService`，server 测试辅助改为显式 owner service，不再通过 `App::list_sessions` / `App::profiles` / `App::agent` 等 helper。
  - 删除 `cfg(test)` 下仅为 server 测试保留的 `AppState.app` / `ServerRuntime.app` 过渡字段。
  - 验证：`cargo test -p astrcode-server --no-run` 与路由级测试在无 `App` shim 情况下通过。
- [x] 6.5.1f-3 删除 server 对 `astrcode-application` 的正式依赖：
  - 分阶段完成：先移除 routes/mapper 对 `application` 摘要类型和请求 DTO 的直接依赖，再抽离 bootstrap/watch/MCP/governance/profile bridge，最后删除 `Cargo.toml` 中的正式依赖并做全量验证。
  - 只有全部子任务完成后，才能视为 `6.5.1f-3` 完成。
- [x] 6.5.1f-3a 收敛 agent route / mapper 的 `application` 摘要类型依赖：
  - 为 server 引入本地 bridge DTO / summary，agent routes 和 mapper 不再直接引用 `AgentExecuteSummary`、`RootExecutionRequest`、`SubRunStatusSummary` 等 `application` 类型。
  - 迁移期允许 `server` 内部 bridge 实现继续调用 `application` 用例，但协议层输入输出必须只依赖 server/core/protocol 类型。
  - 验证：相关 routes / mapper 编译通过，agent/config 路由测试通过。
- [x] 6.5.1f-3b 抽离 bootstrap/watch/MCP/governance/profile bridge 的 `application` 服务依赖：
  - 分阶段完成：先抽离 watch contract，再抽离 profile resolver bridge，随后迁移 MCP bridge，最后处理 governance/runtime status 和通用错误桥接。
  - 只有全部子任务完成后，才能视为 `6.5.1f-3b` 完成。
- [x] 6.5.1f-3b-1 抽离 server-owned watch contract：
  - 将 `WatchSource`、`WatchEvent`、`WatchPort`、`WatchService` 从 `application` 下沉为 server-owned bridge，`bootstrap/watch.rs`、`runtime.rs`、`test_support.rs`、watch 相关路由测试不再直接依赖 `astrcode-application::watch::*` 类型。
  - 迁移期允许 watch 实现内部仍复用 `ApplicationError` 作为错误壳，但 service / source / event / port 类型必须改为 server-owned。
  - 验证：watch 相关测试通过，server bootstrap/watch 代码不再直接引用 `astrcode-application::Watch*` 类型。
- [x] 6.5.1f-3b-2 抽离 server-owned profile resolver bridge：
  - 为 server 引入本地 profile resolver surface，`main.rs`、`runtime.rs`、`agent_api.rs`、测试辅助不再直接暴露 `ProfileResolutionService` 类型。
  - 迁移期允许 bridge 内部继续调用旧 profile resolution 实现，但 server 状态面只暴露本地 contract。
  - 验证：agent/profile/watch 相关测试通过，server runtime/state 不再以 `ProfileResolutionService` 作为公开桥接类型。
- [x] 6.5.1f-3b-3 抽离 server-owned MCP bridge：
  - 将 `McpService`、`McpPort`、`RegisterMcpServerInput`、status summary/view 等 server 直接消费的 bridge 类型下沉到 server-owned contract 或新 owner service。
  - 验证：`bootstrap/mcp.rs`、`main.rs`、MCP routes 不再直接暴露 `astrcode-application::Mcp*` 服务类型。
- [x] 6.5.1f-3b-4 抽离 governance/runtime status 与通用错误桥接：
  - 分阶段完成：先引入 server-owned governance service，再下沉 runtime/config summary projection，最后收敛 `ApplicationError -> ApiError` / conversation error 的通用桥接。
  - 只有全部子任务完成后，才能视为 `6.5.1f-3b-4` 完成。
- [x] 6.5.1f-3b-4a 引入 server-owned governance service：
  - 为 server 引入本地 governance bridge，`main.rs`、`runtime.rs`、composer/config 路由只暴露 server-owned contract，不再以 `AppGovernance` 作为状态面类型。
  - 迁移期允许 bridge 内部继续委托旧治理实现，但 shutdown/reload/runtime snapshot 的公开入口必须经由 server-owned service。
  - 验证：`AppState` / `ServerRuntime` / 测试装配不再暴露 `AppGovernance`。
- [x] 6.5.1f-3b-4b 下沉 runtime/config summary projection：
  - 将 runtime status summary、plugin/capability summary 和 config summary 的协议输入投影下沉为 server-owned projection 类型与函数，`mapper.rs` / `routes/config.rs` 不再直接依赖 `application` 的 summary 类型或 helper。
  - 验证：`mapper.rs`、`routes/config.rs` 不再直接引用 `ResolvedRuntimeStatusSummary`、`RuntimeCapabilitySummary`、`PluginState`、`PluginHealth`、`ResolvedConfigSummary` 或对应的 `application` summary helper。
- [x] 6.5.1f-3b-4c 收敛通用错误桥接：
  - 收敛 `ApplicationError -> ApiError` 与 `ConversationRouteError` 的通用桥接，server 路由错误类型不再通过 `From<ApplicationError>` 作为正式桥接面。
  - 验证：`main.rs`、`routes/config.rs`、`routes/conversation.rs` 不再以 `ApplicationError` 的公共转换实现作为 server 正式错误桥接。
- [x] 6.5.1f-3c 删除 `Cargo.toml` 中的 `astrcode-application` 依赖并做最终验证：
  - 清理残余 `use astrcode_application::*`，移除 `server` crate 对 `astrcode-application` 的正式依赖。
  - 验证：`cargo check --workspace`、路由级冒烟测试和边界检查通过。
- [x] 6.5.1f-3c-1 下沉 config/mode helper 与 validator：
  - 为 server 引入本地 `config`/`mode` helper，`mapper.rs`、`view_projection.rs`、`routes/model.rs`、`routes/sessions/*`、`bootstrap/prompt_facts.rs` 不再直接调用 `application` 的 `resolve_current_model`、`list_model_options`、`is_env_var_name`、`validate_mode_transition`、`format_local_rfc3339` 等 helper。
  - 验证：上述文件不再直接引用这些 `application` helper，相关 route 测试通过。
- [x] 6.5.1f-3c-2 收敛 provider/config bridge 的剩余 `application::config` helper：
  - `bootstrap/providers.rs` 改为消费 server-owned config helper / resolver，避免继续直接依赖 `application::config::*` 常量与 URL/API key helper。
  - 验证：provider 装配代码不再直接引用 `application::config::*` helper。
- [x] 6.5.1f-3c-3 抽离 agent/bootstrap/bridge 的最后一批 `application` 类型：
  - 收敛 `agent_api.rs`、`bootstrap/runtime.rs`、`bootstrap/governance.rs`、`bootstrap/mcp.rs`、`profile_service.rs`、`governance_service.rs`、`mcp_service.rs`、`watch_service.rs` 中残余的 `application` 正式类型依赖，必要时补 server-owned contract。
  - 验证：server 生产代码仅保留迁移内聚实现文件中的最小兼容桥，不再由 runtime/state/routes 暴露 `application` 类型。
- [x] 6.5.1f-3c-3a 引入 server-owned mode catalog bridge：
  - `main.rs`、`bootstrap/runtime.rs`、`routes/sessions/*` 不再以 `application::ModeCatalog` 作为 server 状态面类型，mode 校验与列举改走 server-owned wrapper。
  - 验证：server runtime/state/routes 不再直接暴露 `ModeCatalog`。
- [x] 6.5.1f-3c-3b 引入 server-owned config service bridge：
  - `main.rs`、`bootstrap/runtime.rs`、`http/agent_api.rs`、`bootstrap/providers.rs`、`bootstrap/prompt_facts.rs` 不再以 `application::config::ConfigService` 作为 server 状态面类型。
  - 验证：server runtime/state 对外不再暴露 `ConfigService`。
- [x] 6.5.1f-3c-3c 收敛 root execute / governance assembler bridge：
  - `agent_api.rs` 与 runtime 组装不再直接暴露 `GovernanceSurfaceAssembler`、`execute_root_agent`、`RootExecutionRequest` 等 `application` 执行面类型，必要时补 server-owned execute bridge。
  - 验证：agent route bridge 不再直接暴露 `application` 执行入口类型。
- [x] 6.5.1f-3c-3d 清理剩余 bridge service 的 `application` 内聚实现：
  - 收敛 `profile_service.rs`、`governance_service.rs`、`mcp_service.rs`、`watch_service.rs` 及其 bootstrap 调用面中的残余 `application` 正式类型暴露，只允许最小内部兼容桥留在实现文件。
  - 验证：server 状态/路由/组合根不再把这些 `application` 类型作为正式 surface。
- [x] 6.5.1f-3c-3d-1 下沉 route/main 的 `ApplicationError` 兼容桥：
  - 将 `main.rs`、`http/routes/conversation.rs`、`http/composer_catalog.rs`、`http/agent_api.rs` 中残余的 `ApplicationError -> ApiError/ConversationRouteError` 映射收敛到 server-owned 兼容实现文件，路由与入口文件不再直接依赖 `astrcode_application::ApplicationError`。
  - 验证：上述路由/入口文件不再直接 `use astrcode_application::ApplicationError`。
- [x] 6.5.1f-3c-3d-2 下沉 bootstrap/runtime 对旧 app service 的装配类型：
  - 将 `bootstrap/runtime.rs`、`bootstrap/governance.rs`、`bootstrap/mcp.rs`、`bootstrap/providers.rs` 中残余的 `ModeCatalog`、`McpService`、`ProfileResolutionService`、`AppGovernance` 等旧 app service 装配细节收敛到 bridge builder/impl 文件，组合根只消费 server-owned wrapper。
  - 验证：`bootstrap/runtime.rs` 不再把这些 `application` 类型作为组合根装配 surface。
- [x] 6.5.1f-3c-3d-3 约束 bridge service 内部的 `application` DTO/trait 转换面：
    - 收敛 `mcp_service.rs`、`profile_service.rs`、`governance_service.rs`、`watch_service.rs`、`root_execute_service.rs` 内部残余的 `application` DTO/trait 转换，把旧类型使用限制在最小私有 helper/impl 块中，为 `6.5.1f-3c-4` 的依赖删除做准备。
    - 验证：server 对外 bridge 类型不再暴露 `application` 请求/摘要/trait 作为正式字段或公开参数。
- [x] 6.5.1f-3c-3d-3a 收敛 MCP / watch bridge contract 的 `application` 错误与 service 暴露：
    - 将 `mcp_service.rs` 改为 server-owned port/DTO surface，不再把 `McpService`、`ApplicationError` 或 `astrcode_application::*` 作为正式字段、构造参数或返回类型暴露；`bootstrap/mcp.rs` 只在私有 impl 内做必要转换。
    - 将 `watch_service.rs` 的 `WatchPort` / `WatchService` 错误面切到 server-owned error，`bootstrap/watch.rs` 只在私有 impl 内处理底层错误映射。
    - 验证：`mcp_service.rs`、`watch_service.rs` 的公开 bridge contract 不再直接引用 `astrcode_application::*`。
- [x] 6.5.1f-3c-3d-3b 收敛 governance bridge 的 app snapshot / service 暴露：
    - 收敛 `governance_service.rs` 中残余的 `AppGovernance`、`GovernanceSnapshot` / plugin-entry 依赖，把旧类型限制在私有存储或转换 helper，公开 bridge 输出只使用 server-owned summary。
    - 验证：`governance_service.rs` 的正式字段、公开参数和返回摘要不再直接使用 `astrcode_application` 类型。
- [x] 6.5.1f-3c-3d-3c 收敛 profile / root execute bridge 的 app trait 与 governance-surface 暴露：
    - 分阶段完成：先下沉 profile resolver contract，再下沉 root execute 治理/错误桥，最后清理 builder wiring 中残余的旧 app trait 暴露。
    - 只有全部子任务完成后，才能视为 `6.5.1f-3c-3d-3c` 完成。
- [x] 6.5.1f-3c-3d-3c-1 下沉 profile resolver bridge contract：
    - 将 `profile_service.rs` 的正式字段、构造参数和返回错误面切到 server-owned port / error，不再直接暴露 `ProfileResolutionService` 或 `ApplicationError`；`bootstrap/providers.rs` 只在私有 impl 内保留旧 profile loader / application 兼容。
    - 验证：`profile_service.rs` 的公开 bridge contract 不再直接引用 `astrcode_application::execution::ProfileResolutionService` 或 `ApplicationError`。
- [x] 6.5.1f-3c-3d-3c-2 下沉 root execute governance/error contract：
    - 将 `root_execute_service.rs` 的正式字段、构造参数和返回错误面切到 server-owned port / error，不再直接暴露 `GovernanceSurfaceAssembler`、`ApplicationError`；必要的旧治理装配只允许留在私有 adapter / builder 中。
    - 验证：`root_execute_service.rs` 的公开 bridge contract 不再直接引用这些 `astrcode_application` 类型。
- [x] 6.5.1f-3c-3d-3c-3 清理 private builder wiring 中的旧 app trait 暴露：
    - 收敛 `agent_runtime_bridge.rs`、`bootstrap/providers.rs` 等 builder/wiring 文件中为 profile/root execute bridge 暴露的旧 app trait / service 类型，把它们限制在私有 adapter helper 中。
    - 验证：server 生产 wiring 对外只消费 server-owned profile/root execute contract。
- [x] 6.5.1f-3c-4 删除 `Cargo.toml` 中的 `astrcode-application` 依赖并做最终验证：
  - 清理残余 `use astrcode_application::*`，移除 `server` crate 对 `astrcode-application` 的正式依赖。
  - 验证：`cargo check --workspace`、路由级冒烟测试和边界检查通过。
- [x] 6.5.1f-3c-4a 移除已可直接替换的 `application` re-export / test-only trait 依赖：
  - 将 `main.rs` 中仅作为错误壳使用的 `AstrError` 切到 server/core 自有来源，移除 server tests 中不再需要的 `AppKernelPort` import 与调用习惯。
  - 验证：`main.rs`、`agent_routes_tests.rs`、`session_contract_tests.rs` 不再直接 `use astrcode_application::*`，相关测试通过。
- [x] 6.5.1f-3c-4b 下沉 mode catalog 的剩余 `application` 依赖：
  - 为 server 引入不依赖 `application::ModeCatalog` 的 mode catalog snapshot / transition 校验实现，`mode_catalog_service.rs` 与相关 bootstrap/governance 预览路径不再直接引用 `application::mode::*`。
  - 验证：`mode_catalog_service.rs` 不再直接依赖 `astrcode_application::ModeCatalog` 或 `ModeCatalogSnapshot`。
- [x] 6.5.1f-3c-4c 收敛 config/MCP bridge 的 `application::config` 与 DTO 依赖：
  - 将 `config_service_bridge.rs`、`bootstrap/mcp.rs` 中残余的 `ConfigService`、`RegisterMcpServerInput`、`McpConfig*` application bridge 收敛到 server-owned contract 或私有 compat builder。
  - 验证：server 对外 config/MCP bridge 不再直接暴露这些 `application` 类型。
- [x] 6.5.1f-3c-4d 收敛 governance/bootstrap/app-error 的最终 compat 依赖并删除 crate 依赖：
  - 处理 `application_error_bridge.rs`、`bootstrap/governance.rs`、`bootstrap/runtime.rs`、其余残余 compat imports，完成 `crates/server/Cargo.toml` 中 `astrcode-application` 依赖删除与最终验证。
  - 验证：`crates/server/Cargo.toml` 不再依赖 `astrcode-application`，`cargo check --workspace`、路由级冒烟测试和边界检查通过。
- [x] 6.5.1f-3c-4d-1 下沉 runtime/bootstrap 对 builtin mode seed 与 lifecycle/observability 类型的依赖：
  - 处理 `bootstrap/runtime.rs` 中残余的 `builtin_mode_catalog`、`TaskRegistry`、`RuntimeObservabilityCollector` application import，为 server 提供自有 seed/wrapper/bridge。
  - 验证：`bootstrap/runtime.rs` 不再直接 `use astrcode_application::{ builtin_mode_catalog, RuntimeObservabilityCollector, lifecycle::TaskRegistry }`。
- [x] 6.5.1f-3c-4d-2 收敛 `ApplicationError` 兼容桥与残余测试/runtime-name 耦合：
  - 处理 `application_error_bridge.rs`、`bootstrap/mcp.rs`、相关测试中的 `astrcode-application` runtime name / compat expectation，把 application error 转换限制到最终私有 compat helper。
  - 验证：非私有 compat helper 之外不再直接依赖 `ApplicationError` 或 `astrcode-application` runtime name 常量。
- [x] 6.5.1f-3c-4d-3 收敛 `AppGovernance` compat 并删除 `Cargo.toml` 依赖：
  - 处理 `bootstrap/governance.rs`、`bootstrap/runtime.rs` 与 `crates/server/Cargo.toml` 中残余的 `AppGovernance` / `RuntimeReloader` / `RuntimeGovernancePort` compat，完成 crate 依赖删除与最终验证。
  - 验证：`crates/server/Cargo.toml` 不再依赖 `astrcode-application`，`cargo check --workspace`、路由级冒烟测试和边界检查通过。
- [x] 6.5.2 切换旧 `kernel` 能力面：
  - 将 `CapabilityRouter`、`Kernel`、`KernelGateway`、`SurfaceManager` 的正式调用方迁到 `plugin-host` active snapshot、tool dispatch、provider/resource catalog 或 `agent-runtime` 执行面。
  - 删除 `server` 和新边界中的 `astrcode-kernel` 正式依赖。
  - 分阶段完成：先切掉 route/root-execute 等 agent-control 摘要类型，再切 capability/router 装配与 owner bridge，最后删除 crate 依赖并做最终验证。
  - 只有全部子任务完成后，才能视为 `6.5.2` 完成。
  - 验证：仓库正式代码不再引用 `astrcode_kernel` / `astrcode-kernel`。
- [x] 6.5.2a 收敛 route/root-execute 的 `kernel` agent-control 摘要类型依赖：
  - 将 `http/agent_api.rs`、`root_execute_service.rs`、相关 route/mapper 输出里的 `Kernel`、`SubRunStatusView`、`CloseSubtreeResult` 等正式类型依赖切到 server-owned bridge DTO / service contract。
  - 迁移期允许私有 bridge impl 内部仍调用旧 `kernel` 能力面，但协议层和 server 状态面不能继续暴露这些类型。
  - 验证：agent route、root execute bridge、相关 mapper/summary 不再直接 `use astrcode_kernel::*`。
- [x] 6.5.2b 切换 capability/router 装配到 `plugin-host` snapshot 与 dispatch：
  - 将 `bootstrap/capabilities.rs`、`bootstrap/runtime.rs` 中的 `CapabilityRouter`、`ToolCapabilityInvoker`、surface sync 正式依赖改为 `plugin-host` active snapshot / dispatch / catalog surface，去掉 kernel router 作为 server 正式装配物。
  - 验证：server 组合根与 capability bootstrap 不再以 `CapabilityRouter` 作为正式共享状态。
- [x] 6.5.2c 收敛剩余 owner bridge 对 `KernelGateway` / `SurfaceManager` 的依赖：
  - 处理 `ports/app_kernel.rs`、`ports/agent_kernel.rs`、`ports/session_submission.rs`、`mode/compiler.rs`、`governance_surface/**`、`agent/context.rs`、`execution/subagent.rs` 等残余 owner bridge，把它们迁到 server-owned / plugin-host / agent-runtime / host-session 合同。
  - 验证：server 生产 bridge 与 owner 适配层不再直接暴露 `KernelGateway`、`SurfaceManager` 或其他 `astrcode_kernel` 合同。
- [x] 6.5.2c-1 删除 governance/session submission/agent context 中的死 `CapabilityRouter` / `KernelGateway` 透传：
  - 移除 `ports/session_submission.rs`、`mode/compiler.rs`、`governance_surface/**` 中永远为 `None` 的 router 透传字段与签名，把 root/fresh/resumed governance surface 编译切到纯 mode/runtime 输入。
  - 清理 `agent/context.rs` 中仅用于默认空 limits 的 `KernelGateway` 形参，避免 owner bridge 继续把 gateway 当作正式依赖向上传递。
  - 验证：上述 server bridge/assembler/compile surface 不再直接暴露 `CapabilityRouter` / `KernelGateway`。
- [x] 6.5.2c-2 收敛 `AppKernelPort` / `AgentKernelPort` 的 kernel 合同类型：
  - 将 `ports/app_kernel.rs`、`ports/agent_kernel.rs`、`execution/subagent.rs` 中残余的 `KernelGateway`、`SubRunStatusView`、`CloseSubtreeResult`、`AgentControlError` 等旧 kernel 合同切到 server-owned 最小错误/控制类型，只保留私有 compat impl 调用旧 kernel。
  - 验证：server 生产 port/执行桥不再以这些 `astrcode_kernel` 类型作为正式 trait 字段、参数或返回值。
- [x] 6.5.2c-3 清理剩余 owner bridge / wiring 对旧 kernel compat 的正式暴露：
  - 继续处理 `agent_runtime_bridge.rs`、`main.rs`、相关 owner bridge / 测试装配中的残余 kernel compat 暴露，把正式 surface 收敛到 server-owned contract，为 `6.5.2d` 的 crate 依赖删除做准备。
  - 验证：server 生产 bridge 与组合根不再把旧 kernel compat 类型作为正式对外 surface。
- [x] 6.5.2d 删除 `server` 对 `astrcode-kernel` 的正式依赖并做最终验证：
  - 清理残余 `use astrcode_kernel::*` 与 `Cargo.toml` 依赖，确保 `server` 和新边界不再以旧 `kernel` crate 作为正式依赖。
  - 验证：`cargo check --workspace`、相关路由/组合根测试、`node scripts/check-crate-boundaries.mjs` 通过，且仓库正式代码不再引用 `astrcode_kernel` / `astrcode-kernel`。
- [x] 6.5.3 切换旧 `session-runtime` 剩余调用面：
  - 将旧 `SessionRuntime` 的 session catalog、query/read-model、observe、branch/fork、compaction、turn 提交、child-session 驱动调用全部迁到 `host-session + agent-runtime`。
  - 旧 `session-runtime` 只允许作为迁移源文件存在，不能作为正式 crate 依赖。
  - 验证：仓库正式代码不再引用 `astrcode_session_runtime` / `astrcode-session-runtime`。
- [x] 6.5.3a 收敛 session identity / catalog / fork / 基础 query bridge：
  - 下沉 `session_identity.rs` 等纯 helper，清理 `ports/app_session.rs` 中对旧 `SessionRuntime` catalog/fork/query wrapper 的直接依赖，优先切到 `host-session::SessionCatalog` 与 server-owned 最小摘要类型。
  - 验证：server 基础 session catalog/query bridge 不再因为 session id/fork/catalog helper 正式依赖 `astrcode_session_runtime`。
- [x] 6.5.3b 切换 conversation / terminal query-read-model 面：
  - 将 `http/routes/conversation.rs`、`http/terminal_projection.rs`、相关 route/test 使用的 conversation projector / snapshot / replay DTO 从旧 `session-runtime` 下沉到 server-owned query bridge，并让 durable replay 来自 `host-session`。
  - 验证：conversation / terminal 正式路径不再以 `astrcode_session_runtime` 的 conversation projector / replay DTO 作为正式 surface。
- [x] 6.5.3c 切换 observe / durable collaboration query 面：
  - 将 agent observe、durable subrun status、input queue / parent delivery 恢复等剩余 query 面迁到 `host-session` owner bridge 或 server-owned 最小摘要，避免 server route/agent bridge 继续直接消费旧 `session-runtime` query 类型。
  - 验证：server 协作 query / observe bridge 不再直接暴露 `astrcode_session_runtime` 的 observe/subrun snapshot 类型。
- [x] 6.5.3d 切换 root/subagent submit 与 child-session 驱动面：
  - 将 `execution/root.rs`、`execution/subagent.rs`、`agent_runtime_bridge.rs`、`root_execute_service.rs`、相关 `AppSessionPort` / `AgentSessionPort` 提交入口切到 `host-session + agent-runtime` owner 合同，移除旧 monolith `SessionRuntime` 作为正式 session 提交入口。
  - 验证：server 生产执行桥不再以 `SessionRuntime` 作为 submit / child-session driver 的正式依赖。
- [x] 6.5.3e 删除 `server` 对 `astrcode-session-runtime` 的正式依赖并做最终验证：
  - 清理残余 `use astrcode_session_runtime::*` 与 `Cargo.toml` 依赖，只保留迁移源文件或测试专用引用。
  - 验证：`cargo check --workspace`、相关路由/组合根测试、`node scripts/check-crate-boundaries.mjs` 通过，且 `server` 正式代码不再引用 `astrcode_session_runtime` / `astrcode-session-runtime`。
- [x] 6.5.3e-1 下沉 conversation / session compat DTO 与 projector 依赖：
  - 处理 `conversation_read_model.rs`、`http/terminal_projection.rs`、`http/routes/conversation.rs`、`ports/app_session.rs` 中残余的 `Conversation*Facts`、`SessionReplay`、`SessionTranscriptSnapshot`、`ForkPoint`、stream projector compat，补齐 server-owned DTO / projector bridge。
  - 验证：上述 query/read-model 正式路径不再直接依赖 `astrcode_session_runtime` 的 replay/projector/DTO 类型。
- [x] 6.5.3e-2 下沉 agent control / session submit compat 依赖：
  - 处理 `agent_control_bridge.rs`、`ports/app_kernel.rs`、`ports/agent_kernel.rs`、`ports/agent_session.rs`、`ports/session_submission.rs` 中残余的 `SessionRuntime*` 错误、subrun status、submit payload compat，把旧 runtime 使用限制到最小私有 impl。
  - 验证：server 生产 bridge/port 不再以 `SessionRuntime`、`SessionRuntime*Error`、`SessionRuntimeSubRunStatus`、`astrcode_session_runtime::AgentPromptSubmission` 作为正式 surface。
- [x] 6.5.3e-3 下沉 bootstrap/runtime/governance/capability 装配对旧 runtime 的正式依赖：
  - 处理 `bootstrap/runtime.rs`、`bootstrap/governance.rs`、`bootstrap/capabilities.rs`、`agent_runtime_bridge.rs`、`bootstrap/deps.rs` 中残余的 `SessionRuntime` / `SessionRuntimeBootstrapInput` 装配面，为组合根补齐 host-session + agent-runtime owner bridge。
  - 验证：server 组合根不再把旧 `SessionRuntime` 作为正式共享状态或 bootstrap surface。
- [x] 6.5.3e-4 删除 `Cargo.toml` 中的 `astrcode-session-runtime` 依赖并做最终验证：
  - 清理剩余生产态 `use astrcode_session_runtime::*` 与 `Cargo.toml` 依赖，仅允许测试或迁移源文件保留必要引用。
  - 验证：`cargo check --workspace`、相关路由/组合根测试、`node scripts/check-crate-boundaries.mjs` 通过，且 `server` 生产代码不再引用 `astrcode_session_runtime` / `astrcode-session-runtime`。
- [x] 6.5.3e-4a 引入 server-owned session bridge，收敛 catalog/query/collaboration durable surface：
  - 将 `AppSessionPort` / `AgentSessionPort` 的生产实现从 `SessionRuntime` blanket impl 切到 server-owned bridge；优先使用 `host-session::SessionCatalog` 承接 catalog CRUD、fork、stored-events/query replay、durable subrun/input-queue 协作恢复与 collaboration append。
  - submit / compact / interrupt / observe 等 turn mutation 与 live control 暂允许继续通过私有 legacy runtime compat 调用，直到后续子任务完成。
  - 验证：server bootstrap 不再把 `SessionRuntime` 直接注册为 `AppSessionPort` / `AgentSessionPort` 的正式实现，相关 session contract / route 测试通过。
- [x] 6.5.3e-4b 收敛剩余 session control / observe / transcript compat：
  - 继续处理 `agent_control_bridge_runtime_compat.rs`、`ports/app_kernel_runtime_compat.rs`、`ports/agent_kernel_runtime_compat.rs`、`ports/session_submission_runtime_compat.rs`、session replay / observe 等残余 compat，使 server 正式 owner bridge 不再直接把旧 runtime 类型作为控制面。
  - 验证：server 正式 bridge/port 对外不再以 `SessionRuntime` 或其 query/control helper 作为共享 surface。
- [x] 6.5.3e-4c 删除 `server` crate 对 `astrcode-session-runtime` 的正式依赖并做最终验证：
  - 删除 `crates/server/Cargo.toml` 中的 `astrcode-session-runtime` 依赖，清理残余生产态 `use astrcode_session_runtime::*`，仅保留测试或迁移源文件中的必要引用。
  - 验证：`cargo check --workspace`、相关路由/组合根测试、`node scripts/check-crate-boundaries.mjs` 通过，且 `server` 生产代码不再引用 `astrcode_session_runtime` / `astrcode-session-runtime`。
- [x] 6.5.3e-4c-1 引入 private legacy runtime compat port，收敛 bridge 层的直接 `SessionRuntime` 依赖：
  - 为 server 新增私有 `LegacySessionRuntimePort` compat，把 submit / observe / subrun control / parent delivery 恢复等旧 runtime 能力限制在 compat impl 内，`session_bridge.rs`、`kernel_bridge.rs` 与相关测试 harness 只消费 server-owned trait。
  - 删除 `agent_control_bridge_runtime_compat.rs`、`ports/app_kernel_runtime_compat.rs`、`ports/agent_kernel_runtime_compat.rs`、`ports/session_submission_runtime_compat.rs` 等 blanket compat 模块，避免正式 bridge 继续直接暴露 `SessionRuntime`。
  - 验证：`cargo check -p astrcode-server`、`cargo test -p astrcode-server session_contract_tests -- --nocapture`、`cargo test -p astrcode-server agent_routes_tests -- --nocapture`、`cargo test -p astrcode-server --no-run`、`node scripts/check-crate-boundaries.mjs` 通过，且 `session_bridge.rs` / `kernel_bridge.rs` / `agent/test_support.rs` 不再直接依赖 `SessionRuntime`。
- [x] 6.5.3e-4c-2 收敛 bootstrap owner bridge / keepalive / 测试 runtime handle 的剩余旧 runtime 依赖：
  - 处理 `session_runtime_owner_bridge.rs`、`session_runtime_owner_bridge_compat.rs`、`bootstrap/runtime.rs` 中残余的 `SessionRuntime`、`SessionRuntimeBootstrapInput`、`AgentControlLimits`、`#[cfg(test)] session_runtime` 字段和 keepalive 资源守卫，收敛到 server-owned bootstrap/handle contract 或更小的私有 compat helper。
  - 同步清理仍要求原始 runtime 句柄的生产态 wiring / 测试辅助，让组合根和 owner bridge 不再把旧 runtime 作为正式共享状态或测试输出面。
  - 验证：server 组合根与 owner bridge 的正式字段/装配参数不再直接使用 `astrcode_session_runtime` 类型。
- [x] 6.5.3e-4c-3 删除 `crates/server/Cargo.toml` 中的 `astrcode-session-runtime` 依赖并完成最终收尾：
  - 清理 `legacy_session_runtime_port_compat.rs` 之外残余的 `astrcode_session_runtime` 引用，补齐 `bootstrap/governance.rs`、`bootstrap/capabilities.rs`、`agent/test_support.rs`、`agent/wake.rs` 等测试或迁移源路径的最终替代/下沉。
  - 删除 `crates/server/Cargo.toml` 中的 `astrcode-session-runtime` 依赖，并完成与 `6.5.3e-4` 验证一致的最终检查。
  - 验证：`cargo check --workspace`、相关路由/组合根测试、`node scripts/check-crate-boundaries.mjs` 通过，且 `server` crate 不再正式依赖 `astrcode-session-runtime`。
- [x] 6.5.4 切换旧 `plugin` 进程宿主生产路径：
  - 将 `server/bootstrap/plugins.rs` 和 `governance.rs` 对旧 `PluginLoader`、`Supervisor` 的依赖迁到 `plugin-host` 的等价类型（`PluginLoader`、`ExternalPluginRuntimeHandle` + 补齐 shutdown/health 接口）。
  - SDK 侧（`Worker`、`CapabilityHandler`）和 `examples/example-plugin/` 暂不迁移，后续统一设计新 SDK。
  - 验证：server 生产路径不再引用 `astrcode_plugin` / `astrcode-plugin`。
- [x] 6.5.5 删除旧 crate 与 workspace 依赖：
  - 按真实阻塞面拆分删除，避免 `application` / `session-runtime`、`kernel`、`plugin` 三条尾巴互相阻塞。
  - 只有全部子任务完成后，才能视为 `6.5.5` 完成。
  - 验证：`cargo check --workspace`、`node scripts/check-crate-boundaries.mjs --strict` 通过；仓库中无残留正式依赖路径；最终只剩 `agent-runtime`、`host-session`、`plugin-host` 与共享 `core` 合同。
- [x] 6.5.5a 从 workspace 中移除旧 `application` / `session-runtime` crate：
  - 从根 `Cargo.toml` 和其余活跃 crate 的 `Cargo.toml` 中删除 `application`、旧 `session-runtime` 的正式 workspace 成员与正式依赖。
  - 允许保留源码目录作为迁移归档源，但它们不得再参与 workspace 编译或边界规则判定。
  - 验证：`cargo metadata` / `cargo check --workspace` 中不再出现 `astrcode-application`、`astrcode-session-runtime`。
- [x] 6.5.5b 删除旧 `kernel` crate 的残余 compat / test 正式依赖：
  - 处理 `server` 中残余的 `astrcode-kernel` test-only / compat 依赖，把 `CapabilityRouter`、`Kernel`、`ToolCapabilityInvoker` 等旧类型替换到 server-owned 或 `plugin-host` / `agent-runtime` 最小测试夹具。
  - 删除 `crates/server/Cargo.toml` 与根 workspace 中的 `kernel` 正式依赖/成员。
  - 验证：`cargo check --workspace` 与相关测试通过，且活跃 crate 不再依赖 `astrcode-kernel`。
- [x] 6.5.5c 删除旧 `plugin` crate 与 SDK/example 尾巴：
  - 为 `sdk` / `examples/example-plugin` 提供不依赖旧 `astrcode-plugin` crate 的替代入口或迁移归档策略，再删除根 workspace 与示例中的旧 `plugin` 正式依赖。
  - SDK 侧旧 `Worker` / `CapabilityHandler` 示例不再阻塞宿主边界删除。
  - 验证：workspace 与示例构建路径不再依赖 `astrcode-plugin`。

## 7. 验证与清理

- [x] 7.1 为 `agent-runtime` 补齐测试：turn 执行、provider streaming、tool call/result、hook effect、取消与超时。
- [x] 7.2 为 `host-session` 补齐测试：事件日志恢复、branch/fork、compaction、model_select、child-session 恢复、结果回传、取消传播。
- [x] 7.3 为 `plugin-host` 补齐测试：reload 回滚、in-flight turn snapshot 固定、resource discovery、provider contribution、协作 surface 委托。
- [x] 7.4 全量 CI 检查：`cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib`、`node scripts/check-crate-boundaries.mjs`、`cargo clippy`、`cargo fmt`。
- [x] 7.5 清理过渡桥接代码和迁移痕迹，确保最终仓库只剩新边界与新命名。
