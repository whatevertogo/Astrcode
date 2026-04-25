## 背景

Astrcode 当前的 `session-runtime`、`application`、`server bootstrap` 都承担了过多职责。`session-runtime` 不只是 turn loop，还同时暴露 session catalog、conversation replay、query/read model、child lineage、mode state 等宿主能力；`application` 仍然直接知道 `CapabilityRouter` 和 runtime 提交结构；`server` 则继续手工拼接 builtin tools、agent tools、MCP、plugin、governance、mode catalog 等多套事实源。

这种结构不适合继续往“核心最小化，一切可扩展”演进。即使再叠加 hooks，也只会得到一个更复杂的大核心。与此同时，仓库已经有 hooks 平台与 governance prompt hooks 的历史提案，它们实际上已经证明：Astrcode 更需要的是统一扩展总线，而不是更多 plan/workflow 特判。

本次变更的目标，是把 Astrcode 重构为更接近 `pi-mono` 的分层：核心只保留最小 `agent-runtime`；session 持久化、branch/fork、resource discovery、settings、workflow、governance 等上移到 `host-session` 与 `plugin-host`；builtin 与 external 功能统一通过 plugin / hooks 提供，不再维持多套并行事实源。

同时，`crates/core` 本身也要收缩。它不再继续充当“所有 DTO 和 trait 的总仓库”，而是退回成极薄的共享语义层，只保留真正跨 owner 复用的值对象、消息模型和稳定语义。凡是只被某一个 owner 使用的 DTO、快照、恢复模型、registry、配置或 ports，都应该迁回各自的 owner crate，而不是继续堆在 `core`。

## 目标

- 将运行时核心收缩到新 `agent-runtime` crate，只保留“单 session / 单 agent loop + provider 调用 + tool dispatch + hook dispatch + 流式状态机”这一最小边界。
- 建立新 `host-session` crate，承接事件日志、恢复、branch/fork、session catalog、query/read model 与对外 use-case surface。
- 建立统一的 plugin-first host：builtin 与 external 统一进入同一 registry、active snapshot 与 reload 语义，不再由 server 手工拼接多条特例路径。
- 将正在推进的 hooks 系统直接升级为统一扩展总线，而不是再做一套平行扩展机制。
- 删除 `application` crate，把其职责按新边界重新分配，而不是保留兼容 façade。
- 收缩 `core` crate，只保留真正跨边界共享的语义和最小合同，不再把 owner 专属 DTO 继续放进 `core`。
- 保留并重构多 agent 协作，但继续沿用“一个 session 即一个 agent”的原理：父子 agent 关系、sub-run lineage、输入队列、结果投递与取消语义统一归 `host-session` 管理，`agent-runtime` 只负责最小执行入口。
- 借鉴 `pi-mono` 的 session-as-agent 思路：对外暴露给 LLM、CLI 或扩展的协作能力通过 plugin/tool/command surface 进入系统，但 session durable truth 与协作状态始终由 `host-session` owner 持有。
- 保持 `server` 作为唯一组合根、保持 DTO / 协议层纯数据、保持事件日志优先的持久化原则，但不再让这些约束继续膨胀 runtime core。
- 同步更新 `PROJECT_ARCHITECTURE.md`，让新边界成为仓库级权威约定。

## 非目标

- 不保留旧 crate 结构、旧 API、旧 `application` façade 或旧装配路径的向后兼容壳层。
- 不要求逐字复刻 `pi-mono` 的所有产品能力；本次借鉴的是“最小核心 + 扩展优先”的分层方法，不是照抄它的 Slack、TUI、theme 或 package 生态。
- 不在本 change 内重做前端交互模型；前端只跟随后端新边界做必要适配。
- 不把所有 builtin 功能都强行外置为子进程 plugin；热路径能力允许以内建 plugin 形态存在。

## 变更内容

- **BREAKING**：新建 `agent-runtime` crate，承接当前 `session-runtime` 中的最小 live runtime 核心；旧 `session-runtime` 的“大一统”职责将被拆解，不保留长期兼容壳层。
- **BREAKING**：新建 `host-session` crate，承接事件日志、恢复、branch/fork、session catalog、query/read model 与外部 use-case surface。
- **BREAKING**：多 agent 协作相关的 `SubRunHandle`、父子 session lineage、input queue、subrun finished/cancel 持久化与结果投递，从 `core + application + session-runtime` 的分散结构收敛到 `host-session`；不再保留跨三层拼装的历史布局。
- **BREAKING**：删除 `application` crate。原有用例编排、治理、模式、workflow、MCP、observability 等职责按新边界重分配到 `host-session`、`plugin-host`、`server` 或对应 owner crate，不保留兼容 façade。
- **BREAKING**：删除 `kernel` crate。原先由 `kernel` 承担的 provider/tool/resource 聚合职责拆回 `core` 纯合同、`agent-runtime` 执行面和 `host-session` 装配面，不保留独立门面。
- 新增统一 `plugin-host` 能力层，负责 builtin / external plugin 的注册、active snapshot、reload、resource discovery 与贡献合并。
- 统一 plugin descriptor 到完整贡献面，至少覆盖：
  - `tools`
  - `hooks`
  - `providers`
  - `resources`
  - `commands`
  - `themes`
  - `prompts`
  - `skills`
- 将 hooks 系统升格为统一扩展总线，事件面至少覆盖：
  - `input`
  - `context`
  - `before_agent_start`
  - `before_provider_request`
  - `tool_call`
  - `tool_result`
  - `turn_start`
  - `turn_end`
  - `session_before_compact`
  - `resources_discover`
  - `model_select`
- 将 governance prompt hooks 并入统一 hooks 总线，继续通过既有 `PromptDeclaration` / `PromptGovernanceContext` 链路进入 prompt 组装，不再新增平行 prompt 渲染系统。
- 将 builtin tools、MCP bridge、workflow overlay、governance 行为、resource discovery 等产品能力逐步迁移为 builtin plugins，由统一 registry 与 hooks 总线驱动。
- 将 `spawn_agent`、`send_to_child`、`send_to_parent`、`observe_subtree`、`terminate_subtree` 这类协作能力逐步迁移为 builtin plugin tools/commands；这些 surface 只负责发起协作动作，不持有 collaboration durable truth。
- 更新 `PROJECT_ARCHITECTURE.md` 以及相关 OpenSpec，明确新分层、owner、依赖方向、迁移顺序与失败语义。

## 能力变更

### 新增能力
- `agent-runtime-core`: 定义最小 `agent-runtime` 的边界、输入输出、tool/provider/hook 调度、流式与取消语义。
- `host-session-runtime`: 定义 `host-session` 的事件日志、恢复、branch/fork、session catalog、query/read model 与 host use-case surface。
- `plugin-host-runtime`: 定义统一 builtin / external plugin host、active snapshot、resource discovery、reload 与贡献合并规则。
- `lifecycle-hooks-platform`: 定义统一 hooks 总线、事件分发语义、effect 约束，以及 builtin / external hooks 共享的注册与执行模型。
- `core-boundary-slimming`: 定义 `core` 的收缩边界，移除 owner 专属 DTO、registry、projection、workflow、mode、plugin manifest 和 mega ports。

### 修改能力
- `session-runtime`: 旧的“大一统 session-runtime”被拆解，不再作为单一能力 owner 继续存在。
- `application-use-cases`: 删除 `application` crate，原有 use-case 语义迁移到 `host-session`、`plugin-host` 与 `server` 的新边界。
- `plugin-integration`: 从能力调用桥升级为统一 plugin 贡献面，支持 hooks、providers、resources、prompts、commands 等更宽的扩展面。
- `turn-orchestration`: turn loop 只负责 prompt -> provider -> tool/hook dispatch -> stop/continue，不再直接承载 workflow / governance / discovery 特判。
- `session-persistence`: 事件日志、恢复、branch/fork、read model 的 owner 上移到 host-session 层，与最小 runtime core 解耦。
- `tool-and-skill-discovery`: 资源发现改由 plugin-host / resource discovery 统一驱动，并扩展到 commands、themes、prompts、skills 等完整贡献面。
- `core-semantics`: `core` 不再承载 session 恢复快照、projection、workflow/mode、plugin registry、配置持久化 ports 等 owner 专属内容，只保留共享值对象和稳定语义。

## 影响范围

- 受影响模块
  - `PROJECT_ARCHITECTURE.md`
  - `crates/agent-runtime/*`
  - `crates/host-session/*`
  - `crates/session-runtime/*`
  - `crates/application/*`
  - `crates/kernel/*`
  - `crates/server/src/bootstrap/*`
  - `crates/core/src/agent/*`
  - `crates/application/src/agent/*`
  - `crates/plugin/*`
  - `crates/sdk/*`
  - `crates/core/src/hook.rs`
  - `crates/core/src/plugin/*`
  - `crates/protocol/src/plugin/*`
  - `adapter-*` 中与 builtin tools、MCP、skills、prompt、agents 发现相关的 owner
- 使用方式影响
  - 对用户来说，最终目标是保持“同一产品能力仍可用”，但底层不再区分“核心特判”和“扩展提供”两套实现路径。
  - 对开发者来说，新增内建能力时不再默认改 `application` 或 `server bootstrap` 主链，而是优先通过 builtin plugin / hooks 扩展面接入。
- 架构影响
  - 本次 proposal 与当前 `PROJECT_ARCHITECTURE.md` 对 `session-runtime` 的定义存在冲突，因此必须先更新架构文档，再推进实现。
  - `server` 仍然是唯一组合根，但它的职责会收缩为“装配 `agent-runtime` / `host-session` / `plugin-host` / adapters”，而不是继续作为多套事实源的手工缝合处。

## 旧架构保留内容

以下内容来自旧 `session-runtime` / `core` / `server` 的已验证实现，必须原样或等价迁入新架构，不做重新设计。

### 事件模型（core → host-session 保留）

- `StorageEvent` / `StorageEventPayload` / `StoredEvent`：append-only JSONL 事件体系。
  - 20+ 种事件变体（SessionStart, UserMessage, AssistantDelta/Final, ToolCall/Delta/Result, ToolResultReferenceApplied, PromptMetrics, CompactApplied, SubRunStarted/Finished, ChildSessionNotification, AgentCollaborationFact, ModeChanged, TurnDone, AgentInputQueued/BatchStarted/BatchAcked/InputDiscarded, Error）。
  - `storage_seq` 单调递增，由 session writer 独占分配。
  - `AgentEventContext` 携带 agent 谱系（root / sub-run / fork / resume），支持跨 session lineage 追踪。
  - 校验规则：SessionStart 禁止 turn_id 和 agent 上下文；SubRun 事件要求 child_session_id。
- 这些类型全部保留在 `core`（因为它们是跨 owner 共享的持久化语义），不迁入 host-session。

### 持久化合同（core → host-session 消费）

- `EventLogWriter`：append-only 同步写入器，`append(&mut self, &StorageEvent) -> StoreResult<StoredEvent>`。
- `EventStore`：异步追加，`append(&self, session_id: &SessionId, event: &StorageEvent) -> Result<StoredEvent>`。
- `SessionManager`：会话生命周期管理（create_event_log, open_event_log, replay_events, try_acquire_turn, last_storage_seq, list/delete）。
- `SessionTurnLease`：跨进程 turn 执行租约（RAII 语义，Drop 时释放锁）。
- `SessionTurnAcquireResult`：Acquired / Busy，Busy 时返回当前 turn_id 和 owner_pid，支持自动分叉。
- `FileSystemSessionRepository`：基于文件系统的实现（JSONL + 文件锁）。
- 这些 trait 保留在 `core`；`host-session` 通过 `Arc<dyn EventStore>` / `Arc<dyn SessionManager>` 消费。

### SessionState 投影与广播（session-runtime/state → host-session 迁入）

- `ProjectionRegistry`：增量投影，对每条 StoredEvent 做 `apply()` 更新投影状态（AgentState、turn 投影、child nodes、active tasks、input queue、mode state）。
  - `from_recovery()`：从 checkpoint + tail events 重建完整投影。
  - `snapshot_projected_state()` → `AgentState`（messages, phase, turn_count, mode_id）。
- `SessionState`：组合 projection + writer + 双通道广播。
  - `append_and_broadcast(event, translator)`：append → apply → translate → broadcast，这是事件写入的唯一生产路径。
  - `translate_store_and_cache(stored, translator)`：validate → apply projection → translate to AgentEvent → cache records。
  - 双通道广播：`SessionEventRecord`（durable，含 storage_seq，用于 SSE 断点续传）和 `AgentEvent`（live，token 级 delta 等瞬时事件）。
- `EventTranslator`：StorageEvent → AgentEvent 转换器，按 phase 过滤 sub-run 事件。
- **整块迁入 host-session**，这是 host-session 作为 session truth owner 的核心机制。

### 恢复模型（session-runtime → host-session 迁入）

- `SessionRecoveryCheckpoint`：持久化的恢复快照，包含 `AgentState` + `ProjectionRegistrySnapshot` + `checkpointStorageSeq`。
  - 包含 `childNodes`（ChildSessionNode 索引）、`activeTasks`（任务跟踪）、`inputQueueProjectionIndex`（输入队列投影）。
  - 支持从旧格式 checkpoint 迁移（字段兼容性）。
- 恢复流程：`SessionState::from_recovery(writer, checkpoint, tail_events)` → validate tail events → apply to ProjectionRegistry → cache records → 初始化广播通道。
- **迁入 host-session/recovery.rs**。

### Turn 执行模型（session-runtime/turn → agent-runtime 迁入）

- `run_turn(kernel, TurnRunRequest)` → `TurnRunResult`：turn 主循环。
  - 输入：session_id, working_dir, turn_id, messages, event_store, session_state, cancel token, agent context, prompt declarations, capability router, prompt governance。
  - 循环：`run_single_step()` → `StepOutcome::Continue(transition)` / `StepOutcome::Completed(stop_cause)` / `StepOutcome::Error`。
  - 每步结束后 `flush_pending_events()` 批量写入事件日志。
  - 取消时写入 TurnDone(Cancelled) 后退出。
- `TurnRunRequest` 的装配目前散在 `session-runtime` 和 `application` 中。
- **核心循环迁入 agent-runtime**；TurnRunRequest 的装配（从哪拿 provider、tools、hooks）由 host-session 提供。

### Server 组合根（server/bootstrap → 保留并简化）

- `ServerBootstrapOptions`：可覆盖选项（home_dir, working_dir, plugin_search_paths, enable_profile_watch, watch_service_override），支持测试注入。
- `ServerBootstrapPaths`：从 options 解析路径（config_path, mcp_approvals_path, plugin_skill_root, projects_root, plugin_search_paths）。
- profile watch runtime：监听 agent profile 变更，触发 hot reload。
- MCP warmup：后台任务预热 MCP 连接。
- **保留组合根模式，但内部实现从”手工拼接多套事实源”改为”装配 plugin-host → host-session → agent-runtime”**。

### 其他保留机制

- `ToolSearchIndex`：工具搜索索引，由 adapter-tools 提供。
- `PromptFactsProvider` / `PromptProvider`：prompt 事实来源。
- `GovernanceSurfaceAssembler` / `AppGovernance`：治理面组装。
- `CapabilitySurfaceSync`：能力同步（目前用于 external invokers 变更时同步到 router）。
- config 覆盖层（用户级 → 项目级）、agent profile 解析、mode catalog。

## 新架构详细设计

### host-session 事件日志集成

host-session 作为 session truth owner，通过以下机制接入事件日志：

```
HostSession 持有:
  Arc<dyn EventStore>        ← 由 server 注入
  Arc<dyn SessionManager>    ← 由 server 注入
  sessions: DashMap<SessionId, Arc<LoadedSession>>

LoadedSession 持有:
  SessionState               ← 包含 ProjectionRegistry + SessionWriter + 双通道广播
  SessionActor               ← 消息驱动的 turn 调度

事件写入流:
  agent-runtime 产生 StorageEvent
    → host-session 通过回调收到事件
    → SessionState.append_and_broadcast(event, translator)
      → SessionWriter.append(event)        ← 持久化到 JSONL
      → ProjectionRegistry.apply(stored)   ← 更新投影状态
      → EventTranslator.translate(stored)   ← 转换为 AgentEvent
      → broadcaster.send(record)            ← durable 广播
      → live_broadcaster.send(agent_event)  ← live 广播
```

host-session 对 agent-runtime 只暴露一个事件发射回调，agent-runtime 不直接接触 EventStore 或 SessionWriter。

### host-session 恢复流

```
恢复一个 session:
  1. SessionManager.open_event_log(session_id) → EventLogWriter
  2. SessionManager.last_storage_seq(session_id) → seq
  3. 读取 checkpoint 文件 → SessionRecoveryCheckpoint
  4. 从 checkpoint.checkpointStorage_seq 之后 replay tail events
  5. SessionState::from_recovery(writer, checkpoint, tail_events)
  6. 注册到 HostSession.sessions
```

如果 checkpoint 不存在或损坏，从第一条事件开始全量 replay（现有行为保留）。

### agent-runtime 执行流

```
AgentRuntime.execute_turn(TurnInput) → TurnOutput:
  1. 从 TurnInput 取出 agent-runtime 执行面：
     - session_id, turn_id, agent_id
     - model_ref, provider_ref
     - tool_specs: Vec<CapabilitySpec>     ← 来自 plugin-host active snapshot
     - hook_snapshot_id: String             ← 来自 plugin-host active snapshot
  2. 通过 TurnLoop 循环执行：
     - prompt assembly（消费 prompt_declarations + prompt_governance）
     - provider request（通过 provider_ref 路由到具体 LLM 实现）
     - tool dispatch（通过 tool_specs 匹配，调用 plugin-host 的 dispatch）
     - hook dispatch（通过 hook_snapshot_id 查找注册的 hooks）
     - 每步通过 emit_event 回调发出 StorageEvent
     - stop/continue/continue_with_tool_result 判断
  3. 返回 TurnOutput（session_id, turn_id, agent_id, terminal_kind）
```

agent-runtime 的关键设计：**不持有任何有状态资源**。不持有 EventStore、不持有 plugin registry、不持有 session state。所有有状态依赖通过 TurnInput 传入或通过回调发出。这使得同一个 AgentRuntime 实例可以安全地并发执行多个 session 的 turn。

### server 组合根新设计

```rust
// 新 bootstrap_server_runtime 伪代码
pub async fn bootstrap_server_runtime_with_options(options: ServerBootstrapOptions) -> Result<ServerRuntime> {
    // 1. 路径和配置（保留现有）
    let paths = ServerBootstrapPaths::from_options(&options)?;
    let config_service = build_config_service(paths.config_path)?;
    let resolved_config = config_service.load_overlayed_config(...)?;

    // 2. plugin-host：统一注册表替代手工拼接
    let plugin_host = Arc::new(PluginHost::new());
    let builtin_descriptors = build_builtin_descriptors(&config_service, &paths)?;
    let loader = PluginLoader::new(paths.plugin_search_paths.clone());
    let reload = plugin_host.reload_with_builtin_loader_and_capabilities(
        builtin_descriptors,
        &loader,
        &mcp_capabilities,
    ).await?;
    // reload.snapshot 包含所有 tools/hooks/providers/resources
    // reload.builtin_backends 包含内置插件句柄
    // reload.external_backends 包含外部插件进程

    // 3. host-session：session truth owner
    let event_store: Arc<dyn EventStore> = Arc::new(FileSystemSessionRepository::new_with_projects_root(paths.projects_root));
    let host_session = Arc::new(HostSession::new(event_store, plugin_host.active_snapshot()));

    // 4. agent-runtime：最小执行内核
    let agent_runtime = Arc::new(AgentRuntime::new());

    // 5. 组装完成
    // host_session 持有 event_store + plugin_host 的 active snapshot
    // agent_runtime 通过 TurnInput 获取执行所需的一切
    // server 只负责装配这三者，不再手工拼接多套 invoker

    ServerRuntime { host_session, plugin_host, agent_runtime, governance, handles }
}
```

**核心改变**：旧 `bootstrap_server_runtime` 中手工拼接 core_tool_invokers + agent_tool_invokers + mcp_invokers + plugin_invokers + capability_sync 的 200 行代码，全部被 `plugin_host.reload_with_builtin_loader_and_capabilities()` 一个调用替代。builtin tools、agent tools、MCP tools、plugin tools 全部通过 `PluginDescriptor` 进入同一个 `PluginRegistry`，产出统一的 `PluginActiveSnapshot`。

### 多 agent 协作在新架构中的归属

```
协作行为:
  spawn_child_session  → host-session.HostSession.spawn_child_session()
  send_to_child        → host-session.HostSession.send_to_child()
  send_to_parent       → host-session.HostSession.send_to_parent()
  observe_subtree      → host-session.HostSession.observe_subtree()
  terminate_subtree    → host-session.HostSession.terminate_subtree()

协作 durable truth:
  SubRunHandle         → host-session.collaboration.SubRunHandle
  InputQueueProjection → host-session.input_queue.InputQueueProjection
  SubRunStarted/Finished 事件 → 通过 host-session 的事件日志持久化

协作入口:
  spawn_agent / send_to_child 等作为 builtin plugin tools 注册到 plugin-host
  它们只负责发起动作，不持有 collaboration durable truth

agent-runtime 的职责:
  只执行 child session 的 turn loop
  不感知父子关系、不感知 input queue、不感知协作状态
  所有协作上下文通过 TurnInput.agent 中的 AgentEventContext 传入
```

### hooks 统一扩展总线

```
事件面（来自旧 core::hook + 新增）:
  input                    ← 用户输入到达
  context                  ← prompt 组装前
  before_agent_start       ← turn 开始前
  before_provider_request  ← LLM 请求前
  tool_call                ← 工具调用前后
  tool_result              ← 工具结果返回
  turn_start               ← turn 开始
  turn_end                 ← turn 结束
  session_before_compact   ← compact 前
  resources_discover       ← 资源发现
  model_select             ← 模型选择

注册:
  通过 PluginDescriptor.hooks 注册到 plugin-host
  builtin plugin 和 external plugin 使用相同 HookDescriptor

执行语义:
  顺序分发 / 可取消 / 可拦截 / 可修改 / 管道式 / 短路式
  （借鉴 pi-mono ExtensionRunner 的分发模式）

governance prompt hooks:
  继续通过 PromptDeclaration / PromptGovernanceContext 进入 prompt 组装
  不新增平行 prompt 渲染系统
```

## 约束与风险

- 当前架构权威文档与目标方向冲突，如果不先更新 `PROJECT_ARCHITECTURE.md`，后续实现会持续处于”代码和架构文档互相打架”的状态。
- 项目要求事件日志优先，因此不能简单抛弃 event-sourcing；更合理的做法是把事件日志与 read model 上移到 host-session 层，而不是继续强绑在 runtime core。
- builtin plugin 与 external plugin 必须共享统一 descriptor 和 active snapshot，但不能共享完全相同的性能模型；热路径必须允许进程内 builtin plugin。
- hooks 一旦成为统一扩展总线，就必须明确稳定顺序、失败语义、effect 约束与 observability；否则会把当前特判分支换成更难理解的隐式行为。
- 这是一次大范围 breaking refactor，会同时触及 crate 边界、协议、测试与架构守卫；实现必须按阶段迁移，但阶段迁移不等于保留长期兼容层。
