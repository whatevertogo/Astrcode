# Code Review — dev (vs master)

## Summary
Files reviewed: 264 | New issues: 5 (0 critical, 2 high, 3 medium) | Perspectives: 4/4
Test run: 463 passed, 0 failed

---

## Security

*No security issues found.*

审查范围：shell 工具执行、文件路径处理、HTTP 路由鉴权、MCP 传输安全、LLM provider、插件加载、agent 协作参数校验、workflow 反序列化。所有外部输入路径均有适当校验（白名单 shell family、路径规范化、slug 字符集限制、参数 validate() 方法）。

---

## Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| Medium | `wait_for_turn_terminal_snapshot` 在 broadcaster 关闭后可能自旋 | [watcher.rs:46-54](crates/session-runtime/src/turn/watcher.rs#L46-L54) | 当 broadcast sender 被丢弃且 turn 未到达终态时，`RecvError::Closed` -> resubscribe -> 立即再次 Closed，形成无 yield 的 CPU 自旋循环 |

**Detail**: `subscribe()` 返回的 receiver 在无 sender 时立即 yield `Closed`，`recv().await` 不会让出执行权，形成忙等。需在 resubscribe 后插入 `tokio::task::yield_now()` 或检测 broadcaster 已死并返回错误。

---

## Tests

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| High | `advance_plan_workflow_to_execution()` — planning->executing 关键状态迁移，3 个分支（plan 缺失、plan 未 approved、bridge 缺失）无测试 | [service.rs:54-84](crates/application/src/workflow/service.rs#L54-L84) |
| Medium | `revert_execution_to_planning_workflow_state()` — 反向迁移路径无测试 | [service.rs:86-92](crates/application/src/workflow/service.rs#L86-L92) |
| Medium | `reconcile_workflow_phase_mode()` — 异步 mode 协调，含 3 个分支（phase 匹配、planning 允许 review、switch_mode）无测试 | [service.rs:105-144](crates/application/src/workflow/service.rs#L105-L144) |

**已覆盖**: TurnRuntimeState (6 tests), PostLlmDecisionPolicy (5 tests), WorkflowOrchestrator (5 tests), StreamingJsonTracker, agent module splits.

---

## Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| High | `WorkflowInstanceState` 和 `WorkflowArtifactRef` 在 `application` 与 `adapter-tools` 中各自独立定义，共享同一磁盘文件 `workflow/state.json` | [workflow/state.rs:19-43](crates/application/src/workflow/state.rs#L19-L43), [session_plan.rs:48-71](crates/adapter-tools/src/builtin_tools/session_plan.rs#L48-L71) |

**Detail**: `adapter-tools` 在 `exitPlanMode`/`upsertSessionPlan` 中写入该文件，`application` 在 session bootstrap 时读取。两侧独立定义的 serde struct 一旦漂移（一侧加字段另一侧未同步），将导致静默反序列化失败或数据丢失。应将这两个类型移入 `core`（两 crate 均已依赖 `core`），消除重复。

---

## Must Fix Before Merge

1. **[ARCH-001]** `WorkflowInstanceState` / `WorkflowArtifactRef` 跨 crate 重复定义
   - Impact: 类型漂移导致静默数据丢失
   - Fix: 移入 `core` crate，两侧统一引用

2. **[TEST-001]** `advance_plan_workflow_to_execution()` 关键状态迁移无测试
   - Impact: planning->executing 核心路径无回归保护
   - Fix: 补充 3 个分支的单元测试

---

## Low-Confidence Observations

- `reconcile_workflow_phase_mode` 的 `switch_mode` 失败分支仅 log::warn 后返回错误，调用者是否能正确处理该错误未确认，但不阻塞合并。
