## 1. 合同与文档骨架

- [x] 1.1 在 `crates/application/src/ports/` 新增 `session_contracts.rs`，定义本阶段需要的 app-owned session orchestration contracts（至少覆盖 observe、turn outcome、turn terminal、recoverable parent delivery），并在 `ports/mod.rs` / `lib.rs` 中接好模块导出。验证：`cargo check -p astrcode-application`
- [x] 1.2 更新 `PROJECT_ARCHITECTURE.md`，明确三层分离：外层纯数据快照、中间 durable event truth、内部 runtime control state；并明确 `application` 只依赖稳定 runtime 合同、`session-runtime` 内部 helper 不属于外层合同。验证：`git diff --check -- PROJECT_ARCHITECTURE.md`

## 2. 收紧 application 端口与调用点

- [x] 2.1 修改 `crates/application/src/ports/agent_session.rs`，移除 `ProjectedTurnOutcome`、`TurnTerminalSnapshot`、`AgentObserveSnapshot`、`PendingParentDelivery` 等 runtime/kernel 内部类型泄漏，改为纯数据的 app-owned contracts，并完成 `SessionRuntime` blanket impl 映射。验证：`cargo check -p astrcode-application -p astrcode-server`
- [x] 2.2 修改 `crates/application/src/ports/app_session.rs` 与相关 blanket impl，确保 session-facing port 在本阶段内不再要求调用方理解 runtime 内部规范化/helper 细节。验证：`cargo check -p astrcode-application -p astrcode-server`
- [x] 2.3 修改 `crates/application/src/agent/context.rs`、`crates/application/src/agent/wake.rs`、`crates/application/src/agent/terminal.rs`、`crates/application/src/session_use_cases.rs`、`crates/application/src/test_support.rs`，切换到新 contracts，并删除对 `astrcode_session_runtime::normalize_session_id` 的直接调用。验证：`cargo check -p astrcode-application -p astrcode-server` 与 `rg -n \"astrcode_session_runtime::normalize_session_id|ProjectedTurnOutcome|TurnTerminalSnapshot|AgentObserveSnapshot|PendingParentDelivery\" crates/application/src`
- [x] 2.4 收口 `crates/application/src/lib.rs` 的 orchestration-only runtime re-export，只保留本阶段明确允许继续暴露的稳定 surface。验证：`cargo check -p astrcode-application -p astrcode-server`
- [x] 2.5 检查 `crates/application/src/ports/session_contracts.rs`、`app_session.rs`、`agent_session.rs` 与 `lib.rs`，确保新 contracts 和公开导出不直接承载 runtime control primitives。验证：`rg -n \"CancelToken|AtomicBool|StdMutex|Mutex<|PendingParentDelivery|ProjectedTurnOutcome|TurnTerminalSnapshot|AgentObserveSnapshot\" crates/application/src/ports/session_contracts.rs crates/application/src/ports/agent_session.rs crates/application/src/lib.rs`
- [x] 2.6 复核本次触及的跨 runtime 边界扩展点（至少包括 app-owned session contracts、上层订阅载荷与相关 blanket impl 映射），确保它们遵循“收纯数据、吐纯数据”，不把 runtime-local handle 当作正式合同继续暴露。验证：`rg -n \"HookInput|HookOutcome|PolicyContext|PolicyVerdict|CapabilitySpec|SessionEventRecord\" crates/core crates/application`

## 3. 解开 turn 终态与 compact 事件的重复线

- [x] 3.1 在 `session-runtime` 内提炼共享的 turn terminal projector / outcome helper（放在共享 reducer/projector 模块，而不是 `query/service` 私有逻辑），并让 `crates/session-runtime/src/state/projection_registry.rs`、`src/query/turn.rs` 与 `src/query/service.rs` 共用该实现，删除平行的 `TurnDone` / `Error` 匹配分支。验证：`cargo test -p astrcode-session-runtime query::turn --lib` 与 `cargo test -p astrcode-session-runtime query::service --lib`
- [x] 3.2 把 assistant summary 提取收敛为共享 helper，并修改 `crates/session-runtime/src/turn/submit.rs` 的 subrun finished 构造逻辑复用该 helper，删除 finalize 路径中的局部重复实现。验证：`cargo test -p astrcode-session-runtime turn::submit --lib`
- [x] 3.3 新增 `crates/session-runtime/src/turn/compact_events.rs`（或等价模块），统一主动 / reactive / manual compact 后的 durable 事件组装；修改 `src/turn/request.rs`、`src/turn/compaction_cycle.rs`、`src/turn/manual_compact.rs` 复用共享 builder。验证：`cargo test -p astrcode-session-runtime turn::compaction_cycle --lib` 与 `cargo test -p astrcode-session-runtime turn::manual_compact --lib`
- [x] 3.4 把 `crates/session-runtime/src/turn/replay.rs` 中只读的 transcript/session replay API 迁到 `src/query/replay.rs`（或 `query/transcript.rs` 等价位置），并更新相关导出与调用方。验证：`cargo test -p astrcode-session-runtime query --lib` 与 `rg -n \"session_replay|session_transcript_snapshot\" crates/session-runtime/src/turn`
- [x] 3.5 保持 `crates/session-runtime/src/state/paths.rs` 作为 `session_id` 规范化的唯一所有者，并清理 `crates/session-runtime/src/lib.rs`、`src/query/service.rs`、`src/turn/interrupt.rs`、`src/command/mod.rs` 中绕开 canonical helper 的调用模式。验证：`cargo test -p astrcode-session-runtime state::paths --lib`

## 4. 拉直 turn/state/projection 子域边界

- [x] 4.1 拆分 `crates/session-runtime/src/turn/submit.rs`：保留提交入口与 `TurnCoordinator`，把 finalize / failure / deferred compact 落盘迁到 `src/turn/finalize.rs`（或等价模块），把 subrun started / finished 事件构造迁到 `src/turn/events/subrun.rs`（或等价模块）。验证：`cargo test -p astrcode-session-runtime turn::submit --lib`
- [x] 4.2 移除 `turn` 对 `query` 的反向依赖，把 `current_turn_messages` 等当前 turn 输入读取能力改为 `SessionState` 的直接 API 或 neutral helper；同时让 `interrupt.rs` 不再调用 `submit` 内部 helper 处理 deferred compact。验证：`rg -n \"query::current_turn_messages|submit::persist_pending_manual_compact_if_any\" crates/session-runtime/src/turn`
- [x] 4.3 把 `InputQueueEventAppend` / `append_input_queue_event` 从 `crates/session-runtime/src/state/input_queue.rs` 迁到 `src/command/mod.rs` 或 `src/command/input_queue.rs`，并让 `state/input_queue.rs` 只保留 projection / reducer / 读取逻辑。验证：`rg -n \"InputQueueEventAppend|append_input_queue_event\" crates/session-runtime/src/state`
- [x] 4.4 将 `crates/session-runtime/src/state/projection_registry.rs` 降级为薄协调器，为 turn / children / tasks / input_queue / recent cache 提炼独立 reducer/owner，并把局部 mutation helper 收敛到对应域。验证：`cargo test -p astrcode-session-runtime state --lib`
- [x] 4.5 收口 `crates/session-runtime/src/lib.rs` 的 crate 根导出面，移除不应继续默认暴露给编排层的路径/helper 导出，同时保持本阶段保留的稳定 read-model facts 可用。验证：`cargo check -p astrcode-session-runtime -p astrcode-application -p astrcode-server`
- [x] 4.6 检查 `session-runtime` 对外暴露的 snapshot / result 类型，确认 runtime control state 仍然留在内部实现，不通过新的 façade / contract 外泄。验证：`rg -n \"CancelToken|AtomicBool|ActiveTurnState|TurnRuntimeState|CompactRuntimeState\" crates/session-runtime/src/lib.rs crates/session-runtime/src/query crates/application/src/ports`

## 5. 清理兼容层与回归验证

- [x] 5.1 删除本 change 已完成迁移后不再需要的兼容 re-export / 局部 helper，并确保不新增新的 helper 级跨层调用。验证：`rg -n \"normalize_session_id|append_and_broadcast\" crates/application crates/server`
- [x] 5.2 为新 contracts 映射、turn projector、compact event builder 和 reducer 化后的 projection registry 补回归测试，至少覆盖 observe/outcome/terminal 映射、recovery/live 等价投影和三种 compact 路径的一致事件序列。验证：`cargo test -p astrcode-application --lib` 与 `cargo test -p astrcode-session-runtime --lib`
- [x] 5.3 执行本 change 的完整边界检查与编译验证。验证：`cargo check -p astrcode-session-runtime -p astrcode-application -p astrcode-server`、`node scripts/check-crate-boundaries.mjs`
