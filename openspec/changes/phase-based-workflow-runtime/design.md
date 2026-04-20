## Context

当前系统已经有三套彼此交错但边界不清的机制：

1. **governance mode**
   `code / plan / review` 负责 capability surface、prompt program、child policy 等治理约束。
2. **formal plan workflow**
   `enterPlanMode / upsertSessionPlan / exitPlanMode`、plan prompt 注入、审批解析、plan archive 与前端 surface 共同组成了一条正式工作流，但其编排逻辑散落在 tool、`application`、read model 与前端。
3. **session-runtime live state**
   `SessionState` 同时维护 `phase`、`running`、`active_turn_id`、`turn_lease`、`current_mode`、`child_nodes`、`active_tasks`、`input_queue_projection_index` 等状态；其中一部分是 turn 控制状态，一部分是 durable event 的投影缓存，一部分只是为 query 提供便利的 shadow state。

这三层当前通过约定耦合，而不是通过清晰协议连接，具体表现为：

- `submit_prompt_with_options()` 持有 plan-specific 分支，导致 workflow 行为无法按 phase 复用。
- plan->execute 的衔接依赖 prompt 暗示，`taskWrite` 与 canonical plan 没有显式 bridge。
- `session-runtime` 既维护手写 live 控制状态，又通过 `EventTranslator` / `AgentStateProjector` 维护 display phase 与 mode 投影，存在重复语义。
- `translate_store_and_cache()` 同时承担事件校验、projector 更新、mode/time 缓存更新、recent cache、child projection、task projection、input queue projection 等多项职责，扩展成本高。

仓库规则声明 `PROJECT_ARCHITECTURE.md` 是架构最高参考文档，但当前仓库中未找到该文件。这与文档约束存在直接冲突：本次变更会改动 `application`、`session-runtime`、mode/workflow 分层，必须同步补齐或替代这一权威文档，否则后续实现缺乏统一锚点。

## Goals / Non-Goals

**Goals:**

- 引入显式的 workflow orchestration 层，把“跨 mode 的宏工作流”和“phase 内的小型行为单元”从 plan 特例中抽离出来。
- 保留 mode 作为治理 envelope，但让 workflow phase 成为正式工作流的主轴，使同一 mode 可以被不同 phase 复用。
- 把 `session-runtime` 收敛为两类清晰职责：turn runtime 生命周期控制 + durable event projection。
- 让 plan->execute handoff 从隐式 prompt 约定变成显式 bridge，并继续保持 plan 与 task 两套真相的分层。
- 让 agent turn 从 submit shell 到 runner loop 再到 terminal projection 都有一条明确、可测试、可恢复的 typed 生命周期主脊柱。
- 提供一条可分阶段落地的迁移路径，允许保留现有 plan tool facade 和 metadata schema，避免一次性破坏整个 surface。

**Non-Goals:**

- 不在第一阶段把所有现有 mode 全部改造成可任意组合的 workflow phase。
- 不在第一阶段重写整个前端 transcript/read model surface；除非 workflow 边界需要新增最小事实，否则优先保持现有 conversation contract。
- 不以“通用 hook 框架”替换现有 turn loop；运行时仍使用现有通用 loop、tool cycle、streaming 与 compaction 主路径。
- 不要求第一阶段把所有 workflow 生命周期都改造成完整事件溯源；长文本 artifact 继续沿用文件存储。
- 不在第一阶段把 workflow phase 变更暴露成新的前端 durable event 或 workflow 面板事实；workflow state 仍以内聚的 application 内部状态为主，前端继续围绕既有 mode / transcript / task surface 工作。

## Decisions

### Decision 1：引入 `WorkflowOrchestrator`，由 `application` 持有正式 workflow 编排

新增一层应用级编排器，位于 `application`，负责：

- 装载 `WorkflowDef`
- 读取与写入 session-scoped workflow instance state
- 在提交前解释用户信号（approve / replan / normal）
- 为当前 phase 生成额外 `PromptDeclaration`
- 在 phase 迁移时执行 bridge 逻辑

核心数据模型：

- `WorkflowDef`
  - `workflow_id`
  - `phases: Vec<WorkflowPhaseDef>`
  - `transitions: Vec<WorkflowTransitionDef>`
  - `initial_phase_id`
- `WorkflowPhaseDef`
  - `phase_id`
  - `mode_id`
  - `role`
  - `artifact_kind`（可选）
  - `prompt_overlay`
  - `accepted_signals: Vec<WorkflowSignal>`
  - `exit_gate`
- `WorkflowTransitionDef`
  - `transition_id`
  - `from_phase_id`
  - `to_phase_id`
  - `trigger: WorkflowTransitionTrigger`
- `WorkflowTransitionTrigger`
  - `Signal(WorkflowSignal)`
  - `Auto(WorkflowAutoTrigger)`
  - `Manual(WorkflowManualTrigger)`
- `WorkflowSignal`
  - `Approve`
  - `RequestChanges`
  - `Replan`
  - `Cancel`
- `WorkflowInstanceState`
  - `workflow_id`
  - `current_phase_id`
  - `artifact_refs`
  - `bridge_state`
  - `updated_at`
- `WorkflowBridgeState`
  - `bridge_kind`
  - `source_phase_id`
  - `target_phase_id`
  - `schema_version`
  - `payload: serde_json::Value`

约束：

- `WorkflowTransitionDef` 是 transition 的唯一真相；phase 自身只声明它接受哪些 `WorkflowSignal`，不再在多个 use case 或 tool 入口维护平行 if/else。
- `WorkflowSignal` 是 typed enum，不允许自由字符串直接进入 orchestration 层。自由文本匹配发生在 phase signal interpreter 中，输出必须收敛为 `WorkflowSignal`。
- `WorkflowBridgeState` 采用“稳定 envelope + typed payload”模式：`core` 只定义 envelope；`application` 为 `plan_execute` 提供 `PlanToExecuteBridgeState` 等 typed payload，并序列化进 `payload`。

首个宏工作流定义为 `plan_execute`：

- `planning` phase：绑定 `plan` mode，负责 canonical plan artifact、审批前 gate、自审 checkpoint。
- `executing` phase：绑定 `code` mode，消费 approved plan 的 bridge context，开始执行。
- 后续可增量扩展 `reviewing` 或 `debugging` phase，但不属于本次第一阶段必须交付内容。

**为什么放在 `application` 而不是 `session-runtime`：**

- `application` 已经持有 session 提交入口、治理面装配、plan 审批解析和 prompt declaration 注入。
- `session-runtime` 应保持“执行引擎 + 事件真相 + 读取面”定位，不反向依赖业务工作流。
- 当前 crate 依赖方向也是 `application -> session-runtime`，而不是反过来。
- workflow phase 可能复用同一个 `mode_id`，但不会复用同一套 signal / bridge / artifact 语义，因此 workflow 的主键必须是 phase，而不是 mode。

**备选方案：**

- 让 `ModeId` 直接拥有 workflow 行为：拒绝。一个 mode 可能被多个 phase 复用，若把 workflow 语义绑死到 mode，会再次把 plan/debug/review 等业务规则塞回 mode catalog。
- 把 orchestrator 放进 `session-runtime`：拒绝。会让 runtime 重新承担业务编排，破坏现有依赖方向和职责边界。

### Decision 2：mode 继续负责治理 envelope，phase 负责业务角色与 workflow 语义

mode 的职责保持为：

- capability selector
- action policies
- child policy
- mode prompt program
- execution limits / busy policy

workflow phase 在 mode 之上补充：

- phase role
- artifact 规则
- 用户信号解释
- exit gate
- phase bridge
- 动态 prompt overlay

也就是说，最终一次 turn 的提交上下文由两层叠加：

1. **mode envelope**
   由 governance surface 正常编译。
2. **phase overlay**
   由 `WorkflowOrchestrator` 额外注入 prompt declarations、bridge facts、phase-specific guidance。

这样可以保留现有 mode 编译路径，而不需要引入第二套 prompt/capability 编译系统。

**备选方案：**

- 让 workflow phase 重新编译一整套 capability router：拒绝。会复制 mode 系统已有能力，并让 mode 和 workflow 两套治理入口竞争。
- 让 mode prompt program 直接承载全部 workflow phase 逻辑：拒绝。会把跨 phase 的业务编排重新塞回 mode 定义，失去可组合性。

### Decision 3：`session-runtime` 内部分离“runtime control state”和“display projection state”

保留 `core::Phase` 作为 **display / read-model phase**，继续由 durable event 通过 `EventTranslator` / `AgentStateProjector` 投影得到。

新增内部 runtime lifecycle 模型，用于真正控制执行：

- `TurnRuntimeState`
  - `active_turn: Option<ActiveTurnState>`
  - `compaction: CompactRuntimeState`
- `ActiveTurnState`
  - `turn_id`
  - `cancel`
  - `turn_lease`
  - `stage`
- `TurnRuntimeStage`
  - `Preparing`
  - `RunningModel`
  - `RunningTools`
  - `Finalizing`
  - `Interrupted`
- `CompactRuntimeState`
  - `in_progress: AtomicBool`
  - `failure_count: StdMutex<u32>`
  - `pending_request: StdMutex<Option<PendingManualCompactRequest>>`

`running` 不再作为独立原子真相长期存在，而由 `active_turn.is_some()` 或 runtime stage 派生。但保留一个 lock-free `running` 原子布尔作为缓存镜像，满足 `list_running_sessions()` 等高频读取的零竞争需求。不变式：`running.load() == active_turn.is_some()`，所有写入走 `prepare()` / `complete()` 方法，方法内部同步更新镜像。

`pending_manual_compact: StdMutex<bool>` 被删除，`pending_request.is_some()` 成为唯一待执行 compact 真相。

`TurnRuntimeStage` 与 display `Phase` 的关系是**最终一致的语义对应**，而不是“每次 stage 变更都同步写一次 Phase”。对应关系如下：

- `Preparing` → `Phase::Thinking`
- `RunningModel` → `Phase::Thinking`
- `RunningTools` → `Phase::CallingTool`
- `Finalizing` → 保持进入前的 Phase
- `Interrupted` → `Phase::Interrupted`
- 无 active turn → `Phase::Idle`

但该映射只描述“正常事件流完成后 display Phase 最终会收敛到哪里”，不是 runtime stage 写入时序协议。display `Phase` 仍只由 durable events 经 `PhaseTracker` 推导；`TurnRuntimeStage` 变更不会直接写 `Phase`。

**为什么不直接把现有 `Phase` 扩展为唯一状态机：**

- 当前 `Phase` 已经被 UI、conversation replay、event translation 视为展示语义。
- `Thinking / Streaming / CallingTool` 是面向展示的抽象，不足以表达 lease、cancel、finalizing、deferred compact 等 runtime 互斥控制。
- 把 display phase 与 runtime control state 合并，只会制造新的双重职责。

**备选方案：**

- 继续保留 `phase + running + active_turn_id + lease` 的散落模型：拒绝。隐式不变量过多，难以审计和恢复。
- 让 `AgentStateProjector` 同时承担 runtime control：拒绝。投影器必须保持 pure projection，不应反向成为执行真相。

### Decision 4：以 `ProjectionRegistry` 替换 `translate_store_and_cache()` 中的多重职责

`session-runtime` 内部引入统一 projection registry，把现有投影拆成独立 reducer：

- `phase_tracker`
- `agent_projection`
- `mode_projection`
- `turn_projection`
- `child_session_projection`
- `task_projection`
- `input_queue_projection`
- `recent_record_cache`
- `recent_stored_cache`

`translate_store_and_cache()` 保留为统一入口，但其内部只做：

1. 校验 stored event
2. `projection_registry.apply(stored)`
3. `translator.translate(stored)`
4. 广播 records

当前作为 free function 的 `append_and_broadcast` 收为 `SessionState` 方法，使其内部依次执行写入 event log → `projection_registry.apply()` → `translator.translate()` → 广播。该重构与 `ProjectionRegistry` 引入同步完成。

其中：

- `current_mode_id` 与 `last_mode_changed_at` 不再由 `SessionState` 以额外 shadow state 双写维护，而由 mode projection 统一提供。
- child/task/input-queue 查询都经由 projection registry 的 typed getter 暴露。
- `PhaseTracker` 作为 `ProjectionRegistry` 的一等 reducer 持有 display Phase 真相；它可以额外产出 live `AgentEvent` side effect，因此 reducer 协议允许 reducer 在应用事件时向统一 `ProjectionEffects` 写入附带输出。

`ProjectionRegistry` reducer 协议：

```rust
trait ProjectionReducer {
    fn apply(&mut self, stored: &StoredEvent, effects: &mut ProjectionEffects) -> Result<()>;
}
```

约束：

- reducer 是**有状态对象**，由 `ProjectionRegistry` 持有，而不是纯函数集合。
- reducer 按固定顺序应用：`phase_tracker -> agent_projection -> mode_projection -> turn_projection -> child_session -> task -> input_queue -> recent caches`。
- reducer 之间不直接读写彼此内部状态；共享输入只有 `StoredEvent` 和 `ProjectionEffects`。
- event 在进入 reducer 前已通过 schema 校验，因此 reducer 对合法事件应保持“通常不失败”；若 reducer 仍返回错误，本次内存 apply 终止，并在下次访问时通过 durable replay 重新构建投影。

**备选方案：**

- 保留当前单方法集中更新：拒绝。继续扩展 workflow/read model 时，该方法会持续膨胀。
- 只拆函数不拆投影所有权：拒绝。函数变多但状态归属不清，无法从根上解决边界混乱。

### Decision 8：`TurnExecutionContext` 按内聚性分组，`TurnRuntimeState::complete()` 原子返回 deferred compact

`session-runtime/turn/runner.rs` 中的 `TurnExecutionContext` 当前持有 22 个独立字段，其中多个子集有强内聚关系。在引入 `TurnRuntimeState` 的同时，将 `TurnExecutionContext` 的字段按内聚性分组：

- `TurnLifecycle`：started_at、step_index、continuation_count、last_transition、stop_cause
- `TurnBudgetState`：token_tracker、cache_read_tokens、cache_creation_tokens、auto_compaction_count
- `ToolResultBudgetState`：replacement_state、replacement_count、reapply_count、bytes_saved、over_budget_count
- `StreamingToolState`：launch_count、match_count、fallback_count、discard_count、overlap_ms

这让 `TurnExecutionContext::finish()` 的 summary 收集从 22 个独立字段赋值变成 5 个分组的 `summarize()` 调用。

同时，`TurnRuntimeState::complete()` 在单次调用中完成所有控制状态清理并原子返回 `Option<PendingManualCompactRequest>`，消除当前 `finalize_turn_execution` 中 `complete_session_execution()` 之后再单独调用 `take_pending_manual_compact()` 的悬挂副作用。

**备选方案：**

- 保持 22 个独立字段：拒绝。字段数量随 turn 功能增长会持续膨胀，且 summary 收集逻辑散落。
- 让 deferred compact 通过单独方法在 complete 之后读取：拒绝。引入 complete 和 compact 读取之间的竞态窗口。

### Decision 9：引入 TurnCoordinator 显式收口 turn 生命周期

当前一次 turn 的控制流散落在 `session_use_cases.rs`（accept）、`submit.rs`（prepare + spawn）、`runner.rs`（run）、`execution.rs`（prepare/complete helper）、`submit.rs finalize`（persist + finalize + deferred compact）之间。引入 `TurnCoordinator` 把 `accept → prepare → run → persist → finalize → deferred_compact` 收为单一协调器的显式生命周期方法。

`TurnCoordinator` 是 `session-runtime` 内部的**具体 struct**，不是 trait，也不是 session-scoped 常驻对象。它由 `submit.rs` 在每次 turn 提交时按需创建，持有 `Arc<SessionActor>`、`Arc<SessionState>`、`Arc<Kernel>`、`Arc<dyn EventStore>`、`Arc<dyn RuntimeMetricsRecorder>` 等本轮所需依赖，turn 结束后即释放。它不注册到 `SessionActor` 上，也不成为新的长期状态容器。

`TurnCoordinator` 职责：

- `accept()`：校验请求、解析 busy policy、决定 branch/reject
- `prepare()`：调用 `TurnRuntimeState::prepare()`、持久化 user message 和 queued inputs
- `run()`：委托 `runner::run_turn()` 执行 step 循环
- `persist()`：持久化 turn events、处理 subrun finished event、checkpoint if compacted
- `finalize()`：调用 `TurnRuntimeState::complete()` 原子返回 deferred compact、按需触发 compact

`submit.rs` 简化为请求解析 + `TurnCoordinator::start()` 调用。`runner.rs` 保持为纯 step 循环执行器。`execution.rs` 的 helper 方法合并入 `TurnRuntimeState` 或 `TurnCoordinator`。

**备选方案：**

- 保持当前多模块分段拼装：拒绝。turn 生命周期没有显式协议，新人无法回答“一次 turn 经过哪些阶段”。
- 把所有逻辑合并到 `runner.rs`：拒绝。runner 应保持为纯执行器，不应承担 accept/persist/finalize 编排。

### Decision 10：typed TurnTerminalKind + TurnProjection 替代字符串约定

当前 turn 终态通过 `TurnStopCause → String` → `TurnDone.reason` → 查询侧字符串匹配 传递，语义跨模块漂移。

在 `core` 引入 typed `TurnTerminalKind`：

```rust
enum TurnTerminalKind {
    Completed,
    Cancelled,
    Error { message: String },
    StepLimitExceeded,
    BudgetStoppedContinuation,
    ContinuationLimitReached,
    MaxOutputContinuationLimitReached,
}
```

`TurnDone` durable event 采用兼容演化，而不是直接替换 schema：

```rust
TurnDone {
    timestamp: DateTime<Utc>,
    #[serde(default)]
    terminal_kind: Option<TurnTerminalKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}
```

兼容规则：

- 新写入路径必须写 `terminal_kind`。
- `reason` 保留为兼容镜像字段；迁移窗口内新写入可以继续写 canonical reason code，便于旧读取方工作。
- 反序列化时优先使用 `terminal_kind`；若其缺失，则按 legacy `reason` 映射：
  - `"completed"` → `Completed`
  - `"budget_stopped"` → `BudgetStoppedContinuation`
  - `"continuation_limit_reached"` → `ContinuationLimitReached`
  - `"token_exceeded"` → `MaxOutputContinuationLimitReached`
  - `"cancelled"` / `"interrupted"` → `Cancelled`
  - `"step_limit_exceeded"` → `StepLimitExceeded`
- 旧 `reason` 中的任意未知字符串**不会**直接映射为 `TurnTerminalKind::Error { message }`；error message 只来自 typed `terminal_kind` 或相邻 `Error` event，避免把历史自由文本误解释成新 schema。

终态类型统一计划：

- `TurnTerminalKind` 成为 durable event、query、workflow/parent delivery 读取 turn 终态时的唯一 typed 真相。
- `TurnStopCause` 保留为 runtime 内部 loop 决策原因，并通过显式映射收敛为 `TurnTerminalKind`。
- `TurnOutcome` 从 `TurnRunResult` 中移除，改为直接返回 `terminal_kind`（或等价终态结果对象）。
- `TurnFinishReason` 不再作为独立终态真相，只保留为从 `TurnTerminalKind` 派生的 summary / metrics 视图；最终可在调用方迁完后移除。

`ProjectionRegistry` 扩展包含 `TurnProjection`，记录每个 turn 的终态和摘要。`wait_for_turn_terminal_snapshot()` 等待 `TurnProjection` 到达终态，而不是扫描事件做启发式判断。

**备选方案：**

- 保持字符串约定：拒绝。终态语义漂移会导致查询侧和写入侧对“turn 是否正常完成”判断不一致。
- 只加 enum 不加 TurnProjection：拒绝。不解决查询侧的启发式判断问题。

### Decision 11：PostLlmDecisionPolicy 统一 agent loop 决策层

当前“LLM 返回无工具输出后下一步做什么”分裂在 `continuation_cycle.rs`（输出截断）、`loop_control.rs`（budget auto-continue）、`step/mod.rs`（turn done）三处，靠执行顺序隐式耦合。

引入 `PostLlmDecisionPolicy`，在 step 收到无工具输出后返回 typed 决策：

```rust
enum PostLlmDecision {
    ContinueWithPrompt { nudge: String },
    Stop(TurnStopCause),
    ExecuteTools,
}
```

该 policy 综合考虑：输出截断状态、budget 余量、continuation 计数、step 限制。`step/mod.rs` 的主循环变成可读的决策表：

```rust
match policy.decide(output, step_state, runtime_config) {
    ContinueWithPrompt { nudge } => /* 注入续写提示，继续 step */,
    Stop(cause) => /* 返回 Completed */,
    ExecuteTools => /* 进入 tool cycle */,
}
```

现有 `decide_budget_continuation()` 和 `continuation_cycle` 逻辑合并入 policy。

**备选方案：**

- 保持三处散落逻辑：拒绝。靠执行顺序隐式耦合，修改一处容易破坏另一处的假设。
- 只合并为一个大函数：拒绝。函数会过长且无法独立测试。

### Decision 12：TurnJournal 统一 turn 内部事件记录（低优先级）

当前 durable events 由多个模块直接往 `Vec<StorageEvent>` 推送。引入 `TurnJournal` 作为统一收集器，提升可测试性和可解释性。

`TurnJournal` 不改变现有事件持久化路径，仅替换 `Vec<StorageEvent>` 的直接使用。单个 step 或 cycle 可以通过 `TurnJournal` 验证其产出的事件序列，不需要从 `SessionState` 全局存储中过滤。

本项优先级低于 Decision 9/10/11，可在前三项验证稳定后作为增量补充。

**备选方案：**

- 保持 `Vec<StorageEvent>` 直接使用：可接受但可测试性差。暂不阻塞。
- 让 `TurnJournal` 同时负责持久化：拒绝。增加复杂度且不解决核心问题。

### Decision 13：application-runtime 边界封死，session-runtime 收敛公开 API 面

`session-runtime` 当前直接 re-export `append_and_broadcast`、`prepare_session_execution`、`complete_session_execution` 等低层 helper，`application` 侧（包括 `wake.rs`、`terminal.rs` 的测试代码）已出现直接操控 runtime 内部的迹象。

重构后：

- `session-runtime` 移除对 `append_and_broadcast`、`prepare_session_execution`、`complete_session_execution` 的 re-export
- `application` 只通过 `SessionRuntime` 的公开方法（`submit_prompt`、`switch_mode`、`observe`、query 方法）和 `TurnCoordinator` 生命周期方法消费 runtime
- `application` 不直接接触 execution lease、`EventTranslator`、`Phase` lock 或 event append helper
- `application` 侧测试通过相同的公开 API 面验证行为

**备选方案：**

- 保持低层 re-export 但加注释说明边界：拒绝。注释无法阻止实际使用。
- 引入 facade crate 隔离：过度设计。收敛现有公开方法即可。

### Decision 14：display Phase 完全由事件投影驱动，运行时不再直接变异

当前 `Phase` 存在两条写入路径：`submit.rs` / `execution.rs` 通过 `phase.lock()` 直接写，`PhaseTracker` 通过事件类型推导。重构后消除直接变异路径，让 display Phase 完全由 `ProjectionRegistry` 中的 `PhaseTracker` 事件投影驱动。

`TurnRuntimeStage` 变更不直接写 Phase，也不要求 stage 变更和 phase 事件一一同步发射。`Phase::Streaming`（由 `AssistantDelta` / `AssistantFinal` 触发）和 `Phase::CallingTool`（由 `ToolCall` 触发）继续由 `PhaseTracker` 事件推导。`TurnRuntimeStage` 只影响 runtime control，display Phase 通过事件流**最终一致地**收敛。

`PhaseTracker` 被纳入 `ProjectionRegistry`，成为和 mode/task/input-queue 同级的 reducer，而不是散落在翻译器与 runtime helper 之间的第二套 phase 真相。

**备选方案：**

- 保持 `phase.lock()` 直接写：拒绝。两条写入路径会在 Streaming/CallingTool 等中间态产生冲突。
- 让 `TurnRuntimeStage` 完全包含 Phase 语义：拒绝。Phase 有 `Streaming`、`Done` 等纯展示态，不对应任何 runtime stage。

### Decision 15：interrupt 和 fork 走 TurnCoordinator，不绕过协调器

`interrupt_session()` 和 `fork_session()` 当前直接操作散落字段（`running`、`active_turn_id`、`cancel`），与 `TurnCoordinator` 封装的 lifecycle 不一致。重构后：

- `interrupt_session()` 通过 `TurnCoordinator::interrupt()` 触发，`interrupt()` 调用 `TurnRuntimeState` 的 transition API
- `fork_session()` 通过 `TurnCoordinator` 读取 turn 状态（stage、turn_id），不直接读取散落字段
- `resolve_submit_target()` 的 branch 逻辑通过 `TurnRuntimeState::running()` 缓存镜像判断

**备选方案：**

- 让 interrupt/fork 保持直接操作：拒绝。绕过协调器会让 lifecycle 状态回到不一致。

### Decision 16：TurnRuntimeState 崩溃恢复重置为无活跃 turn

`normalize_recovered_phase()` 把 display Phase 从 `Thinking/Streaming/CallingTool` 降级为 `Interrupted`，但 runtime control state 没有相应的恢复语义。引入 `TurnRuntimeState` 后：

- 恢复时 runtime control state 初始化为无 active turn（`active_turn: None`，`running: false`）
- 崩溃前的 turn 不可恢复，display Phase 由 `normalize_recovered_phase()` 映射为 `Interrupted`
- 不需要从 event log 中尝试恢复 runtime control state（lease、cancel 等都是进程内的）

**备选方案：**

- 尝试从 event log 恢复 runtime control state：拒绝。lease 和 cancel token 是进程内状态，不可序列化恢复。

### Decision 5：保留 artifact 文件存储，但把 workflow instance state 显式化

长文本 artifact（如 canonical plan）继续使用 session 目录下的文件存储；新增 workflow instance state 也使用 session-scoped 显式状态文件，而不是隐式内存状态。

建议目录：

```text
.astrcode/sessions/<session-id>/workflow/
  state.json
  phases/
    planning/
      artifact.md
      artifact-state.json
```

第一阶段不要求把 workflow phase transitions 全量事件化，原因如下：

- 当前 canonical plan 已经是文件存储模式，完整迁到事件日志会显著扩大改动面。
- 长文档内容不适合直接进入高频 session event log。
- 本次更优先解决“编排与状态边界清晰”，而不是一次性把 artifact 存储范式全部替换。

但需满足以下约束：

- workflow instance state 必须是显式持久化状态，不能只存在内存。
- session event log 仍然是 turn、mode change、tool result、task snapshot 等执行时间线的唯一 durable timeline。
- workflow state 文件缺失或损坏时，系统将该 session 视为没有 active workflow，降级到 mode-only 路径，并记录包含损坏路径的警告日志。
- workflow state 的恢复独立于 `SessionRecoveryCheckpoint`：先按既有 runtime 恢复顺序恢复 session actor 和投影，再由 `application` 单独加载 workflow instance state。workflow state 损坏不会阻塞 session 恢复；checkpoint 损坏则按 runtime 自己的恢复策略处理，与 workflow state 不建立原子事务。
- 第一阶段 workflow state 仅对 `application` 内部可见，不新增 `WorkflowPhaseChanged` durable event 或前端面板事实。若后续需要 workflow-aware UI，单独发起 change。

**备选方案：**

- 全量事件溯源 workflow phase state：暂不采用。长期方向可保留，但第一阶段成本过高。
- 继续复用 plan state.json 而不引入 workflow state：拒绝。会继续把 workflow 真相绑死在 plan 特例上。

### Decision 6：plan->execute handoff 使用显式 bridge，但不自动生成 `taskWrite` durable snapshot

phase bridge 只负责把 source artifact 的关键信息转成 target phase 可消费的上下文，不直接伪造执行期 durable truth。

`PlanToExecuteBridge` 的职责：

- 解析 approved plan 的 `Implementation Steps`
- 生成 execute phase prompt overlay
- 指示模型把步骤转成执行顺序与 `taskWrite` 计划

它**不直接**做的事：

- 不直接写入 task snapshot
- 不修改 canonical plan artifact
- 不把 task panel 作为 plan 的派生视图

这样可以继续满足“task 与 canonical plan 是两套真相”的现有原则。

**备选方案：**

- 审批后系统自动写入 `taskWrite` snapshot：拒绝。会让 task durable truth 不再完全来自 task 系统。
- execute phase 完全靠旧 prompt 暗示理解 plan：拒绝。交接语义不可测试，也无法清晰支持 replan。

### Decision 7：运行时内部采用 reducer / typed transition，业务层采用 signal / orchestrator，不引入统一“大 hook 框架”

本次不会把 `core::hook` 扩展成所有内部行为的统一机制。

分层策略：

- `session-runtime` 内部：使用 reducer / typed transition
  - 负责 turn runtime state 与 projections
- `application`：使用 workflow signal / orchestrator
  - 负责解释用户输入、工具结果、phase bridge 与迁移

与现有 `HookHandler` 系统的边界：

- `HookHandler`（`PreToolUse` / `PostToolUse` / `PreCompact` / `PostCompact`）：粒度为单次工具调用或压缩，面向插件扩展。
- `WorkflowOrchestrator`：粒度为 turn 提交边界，面向业务编排。
- 两者不在同一层竞争：hook 不感知 workflow phase，workflow 不直接消费 hook 结果。

原因：

- runtime 内部变化需要高频、强类型、低开销，不适合走通用 hook 分发。
- workflow 编排需要业务语义和 phase 上下文，天然适合 signal/orchestrator 模型。

**备选方案：**

- 构建统一 hook runtime 让 runtime/application/tool/frontend 都挂进去：拒绝。抽象太大，容易重新制造第二套事实来源。

### Decision 17：`SessionRecoveryCheckpoint` 收敛为 projection registry 快照，而不是旧字段并行表

现有 `SessionRecoveryCheckpoint` 持有顶层 `phase`、`last_mode_changed_at`、`child_nodes`、`active_tasks`、`input_queue_projection_index` 等字段；在 `ProjectionRegistry` 引入后，这种结构会把 checkpoint 变成第二套投影真相。

重构后：

- 顶层 `phase` 字段删除；display Phase 只通过 `agent_projection/phase_tracker` 快照恢复。
- 顶层 `last_mode_changed_at` 字段删除；mode 时间戳由 `mode_projection` 快照恢复。
- child/task/input-queue/turn 等恢复数据通过 `ProjectionRegistrySnapshot` 统一持有，而不是 checkpoint 顶层继续平铺一组特例字段。
- 为兼容既有 checkpoint，恢复路径接受旧 schema：缺失 `projection_registry` 时，从旧顶层字段构造等价的 reducer snapshot；新写入路径只写新 schema。

这样 recovery 只有一份“投影快照真相”，不会继续并行维护旧 checkpoint 字段和新 registry 状态。

### Decision 18：workflow phase 迁移以 workflow state 为主记录，并采用 best-effort reconcile

phase 迁移涉及：解释 signal、执行 bridge、写 workflow state、切 mode、生成 overlay。它无法跨文件系统和 event log 做强事务，因此本次采用明确的主记录和补偿策略：

1. 先验证 signal 与 transition。
2. 计算 bridge 输出并原子写入新的 `WorkflowInstanceState`（主记录）。
3. 再通过现有 mode 切换入口写 `ModeChanged` durable event。
4. prompt overlay 只在本次提交上下文中生效，不属于 durable truth。

失败处理：

- 若 workflow state 写入失败，则 phase 迁移失败，mode 不切换。
- 若 workflow state 写入成功但 mode 切换失败，则保留新的 workflow phase，并在下一次提交或恢复时按 `current_phase_id -> mode_id` 进行 reconcile；因为 phase 对 mode 是单向可推导的，而 mode 不能可靠反推 phase。
- bridge 失败视为 phase 迁移失败，不写 workflow state，也不切 mode。

这样可以避免“mode 已切但 workflow 仍停在旧 phase”的不可恢复歧义。

## Risks / Trade-offs

- **[Risk] workflow 与 mode 边界定义不清，可能再次出现双重职责**
  → Mitigation：在设计与 specs 中明确“mode 负责治理 envelope，phase 负责业务角色与迁移”，实现阶段禁止 workflow 自行编译 capability router。

- **[Risk] runtime control state 与 display phase 过渡期并存，短期内代码会更复杂**
  → Mitigation：分阶段迁移，先引入 `TurnRuntimeState` 与 transition API，再逐步移除 `running` 和直接 `phase.lock()` 写法。

- **[Risk] workflow state 文件与 event log 双持久化带来一致性风险**
  → Mitigation：限定两者职责；workflow state 只记录 phase/orchestrator truth，event log 继续记录执行 timeline；所有 workflow 文件写入都必须通过统一 application service，并由 phase->mode reconcile 处理 mode switch 失败场景。

- **[Risk] 现有 plan tool / metadata / frontend surface 兼容层会延长过渡期**
  → Mitigation：第一阶段显式保留 facade，并为兼容层设定退出条件：当第二个 workflow 跑通且 conversation/read model 已支持泛化 surface 后，再统一重命名。

- **[Risk] `PROJECT_ARCHITECTURE.md` 缺失会让实现阶段失去跨 PR 的统一约束**
  → Mitigation：把补齐或替代该权威文档作为本 change 的显式任务之一，先落文档，再大规模重构。

- **[Risk] plan->execute bridge 做得过强会侵入 task truth，做得过弱又会回到 prompt 暗示**
  → Mitigation：bridge 只注入执行上下文，不直接写 task durable snapshot；是否自动初始化 task 面板留作后续单独 change 决策。

## Migration Plan

1. 在 `core` 引入 workflow/phase/signal 的纯数据协议、typed turn terminal schema 和 checkpoint 演化策略，并补齐架构文档。
2. 在 `application` 增加 `WorkflowOrchestrator` 与 `plan_execute` 定义，但先保持现有 plan tool facade 与 metadata schema。
3. 在 `session-runtime` 引入 `TurnRuntimeState` 与 `ProjectionRegistry`，把 `SessionState` 的控制状态和投影状态分组收敛，同时保证 query contract 不变。
4. 把 `submit_prompt_with_options()` 的 plan 特判迁移到 orchestrator，使用 phase overlay 注入 prompt，用 signal 处理 approval / replan。
5. 引入 `PlanToExecuteBridge`，让 execute phase 明确消费 approved plan 上下文，但继续由 `taskWrite` 维护 task durable truth。
6. 用第二个 workflow（例如 debug/investigate 类）验证抽象，再决定是否泛化现有前端 surface 和工具命名。

回滚策略：

- 若 orchestrator 路径不稳定，可恢复到旧的 plan-specialized submit path，同时保留已经验证行为等价的 runtime state/projection 收敛改动。
- 若 runtime lifecycle 重构导致恢复或查询异常，可先退回旧 `SessionState` 字段布局，但保留新 specs 与架构文档，避免再次回到“无边界”的实现方式。

## Open Questions

- `PROJECT_ARCHITECTURE.md` 应作为新文件补齐，还是已有中文文档中已有等价权威来源但命名未统一？需要在实现前确认。
- 第二个验证抽象的 workflow 选择 `debug -> fix -> verify` 还是 `investigate -> plan -> execute`，需要在 tasks 排序时最终确定。
