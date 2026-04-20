## 1. 架构锚点与共享协议

- [ ] 1.1 补齐 `PROJECT_ARCHITECTURE.md` 或统一现有等价架构文档引用，明确 `mode envelope / workflow phase / session-runtime` 三层分工与依赖方向；验证：`rg -n "workflow|phase|session-runtime|application" PROJECT_ARCHITECTURE.md docs`
- [ ] 1.2 在 `crates/core/src/workflow.rs`（及 `crates/core/src/lib.rs`）定义 `WorkflowDef`、`WorkflowPhaseDef`、`WorkflowTransitionDef`、`WorkflowTransitionTrigger`、`WorkflowSignal`、`WorkflowBridgeState` 等纯数据协议，显式包含 transition/source/target/trigger 与 bridge envelope 字段；补充序列化/默认值测试；验证：`cargo test -p astrcode-core --lib`
- [ ] 1.3 在 `crates/core/src/event/types.rs` 引入 typed `TurnTerminalKind`，为 `TurnDone` 增加兼容字段 `terminal_kind: Option<TurnTerminalKind>`，保留 legacy `reason: Option<String>` 作为迁移镜像；实现旧事件 `reason` 到 typed terminal 的反序列化映射，并补充 serde 兼容测试；验证：`cargo test -p astrcode-core --lib`
- [ ] 1.4 收敛 turn 终态类型：让 `TurnTerminalKind` 成为 durable/query 真相，`TurnStopCause` 只保留 runtime 内部用途，`TurnOutcome` 与 `TurnFinishReason` 移除或改为从 `TurnTerminalKind` 派生；验证：`cargo test -p astrcode-core --lib` 与 `cargo test -p astrcode-session-runtime --lib`

## 2. session-runtime 生命周期收敛

- [ ] 2.1 在 `crates/session-runtime/src/state/` 引入 grouped runtime state（如 `TurnRuntimeState`、`ActiveTurnState`、`CompactRuntimeState`），替换 `running`、`cancel`、`active_turn_id`、`turn_lease` 的散落写法；`CompactRuntimeState` 用 `pending_request.is_some()` 替代独立 `pending_manual_compact` 布尔；`running` AtomicBool 保留为 `active_turn.is_some()` 的 lock-free 缓存镜像；恢复时 `TurnRuntimeState` 重置为无活跃 turn；验证：新增/更新 `state` 单测并运行 `cargo test -p astrcode-session-runtime --lib`
- [ ] 2.2 在 `crates/session-runtime/src/state/` 实现 `ProjectionRegistry` 与 stateful reducer 协议，把 `phase_tracker`、`agent_projection`、`mode_projection`、`turn_projection`、child session、active tasks、input queue、recent cache 的更新从 `translate_store_and_cache()` 中拆成独立 reducer；将 `append_and_broadcast` free function 收为 `SessionState` 方法，使其内部依次执行写入 → `projection_registry.apply()` → `translator.translate()` → 广播；验证：`cargo test -p astrcode-session-runtime --lib`
- [ ] 2.3 演化 `SessionRecoveryCheckpoint`：移除顶层 `phase` 与 `last_mode_changed_at`，改为持有 projection registry 快照；保留旧 checkpoint 兼容恢复路径，并补齐 old-checkpoint → new-snapshot 的回归测试；验证：`cargo test -p astrcode-session-runtime --lib`
- [ ] 2.4 修改 `crates/session-runtime/src/turn/submit.rs`、`turn/interrupt.rs`、`state/execution.rs` 与相关 query 路径，统一通过显式 runtime lifecycle transition API 推进 turn，移除直接 `phase.lock()` 和分散 reset 逻辑；`TurnRuntimeState::complete()` 原子返回 `Option<PendingManualCompactRequest>`，消除 `finalize_turn_execution` 中 complete 后的悬挂副作用调用；验证：`cargo test -p astrcode-session-runtime --lib`
- [ ] 2.5 引入 per-turn 具体 `TurnCoordinator`，把 `accept → prepare → run → persist → finalize → deferred_compact` 收为单一协调器的显式生命周期方法；`submit.rs` 简化为请求解析 + `TurnCoordinator::start()`；`execution.rs` helper 合并入 `TurnRuntimeState` 或 `TurnCoordinator`；`interrupt_session()` 和 `fork_session()` 走 `TurnCoordinator`；`TurnCoordinator` 使用 `generation: AtomicU64` 防护 interrupt/resubmit 竞态：`prepare()` 递增 generation，`complete(generation)` 仅在匹配时执行清理，`interrupt()` 无条件递增并清理；补齐 interrupt-then-resubmit 竞态回归测试；验证：`cargo test -p astrcode-session-runtime --lib`
- [ ] 2.6 在 `crates/session-runtime/src/turn/runner.rs` 将 `TurnExecutionContext` 的 22 个字段按内聚性分组为 `TurnLifecycle`、`TurnBudgetState`、`ToolResultBudgetState`、`StreamingToolState` 等子结构，让 `finish()` 的 summary 收集从逐字段赋值变成分组 `summarize()` 调用；验证：`cargo test -p astrcode-session-runtime --lib`
- [ ] 2.7 引入 `PostLlmDecisionPolicy`，合并 `continuation_cycle.rs`、`loop_control.rs::decide_budget_continuation`、`step/mod.rs` 中”无工具输出后下一步”的散落逻辑；step 主循环变成 `match policy.decide()` 的决策表；policy 包含收益递减检测（`continuation_count` 超阈值且最近 k 次 output 偏低时提前终止）；验证：新增 policy 单元测试并运行 `cargo test -p astrcode-session-runtime --lib`
- [ ] 2.8 扩展 query 路径使用 `TurnProjection` 读取终态，替换 `wait_for_turn_terminal_snapshot()` 对事件扫描与字符串 reason 匹配的依赖；补齐 legacy `TurnDone.reason` 与 typed `terminal_kind` 混合历史的回归测试；验证：`cargo test -p astrcode-session-runtime --lib`
- [ ] 2.9（低优先级）引入 `TurnJournal` 替换 `Vec<StorageEvent>` 直接使用，让单个 step/cycle 可通过 journal 验证其事件序列；不改变现有事件持久化路径；验证：`cargo test -p astrcode-session-runtime --lib`

## 3. application workflow orchestration

- [ ] 3.1 新增 `crates/application/src/workflow/` 模块，实现 `WorkflowOrchestrator`、`plan_execute` workflow 定义、session-scoped workflow state 读写服务，以及 `PlanToExecuteBridge` 所需的 typed 状态结构；验证：`cargo check --workspace`
- [ ] 3.2 实现 workflow state 的独立恢复与降级策略：session-runtime 恢复完成后再加载 workflow state；workflow state 损坏时降级到 mode-only 路径；补齐恢复测试；验证：`cargo test -p astrcode-application --lib`
- [ ] 3.3 定义 phase transition 的持久化边界：先原子写 `WorkflowInstanceState`，再切换 mode；若 mode 切换失败，则按 `current_phase_id -> mode_id` 在后续提交/恢复时 reconcile；补齐失败补偿测试；验证：`cargo test -p astrcode-application --lib`
- [ ] 3.4 重构 `crates/application/src/session_use_cases.rs`，让提交入口先经由 orchestrator 解释 active workflow、phase overlay 与用户信号，再编译 governance surface；保留“无 active workflow 时回退到现有 mode-only 路径”的行为；验证：新增应用层 orchestration 测试并运行 `cargo test -p astrcode-application --lib`
- [ ] 3.5 收敛 `crates/application/src/session_plan.rs` 为 planning phase 的 artifact/service facade，保留当前 canonical plan 路径、archive 语义与对外工具 contract，但把审批、phase 迁移和 bridge 触发迁入 orchestrator；验证：`cargo test -p astrcode-application --lib`

## 4. plan_execute bridge 与任务边界

- [ ] 4.1 调整 `crates/adapter-tools/src/builtin_tools/enter_plan_mode.rs`、`upsert_session_plan.rs`、`exit_plan_mode.rs`，使其内部委托新的 planning phase service / workflow state，而对外继续保留现有工具名与 metadata schema；验证：`cargo test -p astrcode-adapter-tools --lib`
- [ ] 4.2 在 `crates/application` 与 `crates/session-runtime` 增加回归测试，确认 approved plan 进入 executing phase 时只生成 bridge context，不会隐式创建 `taskWrite` snapshot；验证：`cargo test -p astrcode-application --lib` 与 `cargo test -p astrcode-session-runtime --lib`
- [ ] 4.3 增加 `replan` 回路的应用层测试，确认 executing -> planning 迁移不会隐式清空现有 active task snapshot，task durable truth 仍只由 `taskWrite` 驱动；验证：`cargo test -p astrcode-application --lib`

## 5. 兼容层清理与整体验证

- [ ] 5.1 清理过时的 plan-specific submit helper、重复 shadow state 写入与无主状态访问路径；移除 `session-runtime` 对 `append_and_broadcast`、`prepare_session_execution`、`complete_session_execution` 的 re-export；确保 `application` 不直接接触 execution lease、`EventTranslator`、`Phase` lock 或 event append helper，只通过 `SessionRuntime` 公开方法或 `TurnCoordinator` 消费；`application` 侧测试通过相同公开 API 面验证行为；验证：`node scripts/check-crate-boundaries.mjs` 与 `cargo check --workspace`
- [ ] 5.2 若 workflow-aware plan surface 对 conversation/tool display 测试有影响，更新 `crates/session-runtime/src/query/conversation/*`、`frontend/src/lib/toolDisplay.ts`、`frontend/src/components/Chat/*` 的兼容测试与最小实现，保持现有 facade 稳定；第一阶段不新增 workflow-aware durable event 或前端面板事实；验证：`cargo test -p astrcode-session-runtime --lib`、`cd frontend && npm run typecheck && npm run lint`
- [ ] 5.3 运行整体验证并修正遗留问题：`cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib`、`node scripts/check-crate-boundaries.mjs`、`cd frontend && npm run typecheck && npm run lint && npm run format:check`
