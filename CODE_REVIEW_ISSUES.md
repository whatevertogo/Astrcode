# Code Review — dev (572bd0a0)

## Summary
Files reviewed: ~45 Rust source files (core, session-runtime, application, server) | New issues: 6 (0 critical, 1 high, 3 medium, 2 low) | Perspectives: 4/4

---

## 🔒 Security

*No security issues found.*

所有新增输入面（`submit_prompt_inner`、`compact_session`、`WorkflowStateService`）均通过内部可信路径调用，不直接暴露给外部 HTTP 端点。`session_id` 经过 `normalize_session_id` 处理，`working_dir` 通过 `project_dir` 校验。无硬编码 secret、无注入路径。

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| High | `persist_deferred_manual_compact` 中 `set_compacting(true)` 无 finally-guarantee | `session-runtime/src/turn/finalize.rs:93-105` | 若 `build_manual_compact_events` panic，`compacting` 标志永远不会复位 |
| Medium | `subrun_finished_event` 硬编码中文 fallback 消息到 durable event | `session-runtime/src/turn/subrun_events.rs:47` | 事件数据耦合中文，不利于国际化或外部消费 |
| Medium | `wait_for_turn_terminal_snapshot` 无内置超时，可能无限等待 | `session-runtime/src/turn/watcher.rs:26-56` | 若 turn 永远不终止（session 被删除等），调用者无限挂起 |
| Low | `ProjectionRegistry::apply` 每次事件都 clone turn_id | `session-runtime/src/state/projection_registry.rs:343` | 事件回放场景下的不必要的 String 分配 |

### [QUAL-001] High: `set_compacting(true)` 无 panic-safe 保护

`finalize.rs:93-105`:

```rust
turn_runtime.set_compacting(true);       // ← 设置标志
let built = build_manual_compact_events(...).await;  // ← 如果 panic?
turn_runtime.set_compacting(false);      // ← 永远不会执行
```

如果 `build_manual_compact_events` panic（如 LLM provider 返回非预期数据导致 unwrap），`compacting` 标志将永远为 `true`，阻止后续所有 manual compact 请求。

**Fix**: 使用 RAII guard 或 `scopeguard`/`defer` 模式确保 `set_compacting(false)` 总是执行：

```rust
let _guard = scopeguard::guard((), |_| turn_runtime.set_compacting(false));
let built = build_manual_compact_events(...).await;
```

注意：同样的问题也存在于 `command/mod.rs:163-177` 的 `compact_session` 方法中。

### [QUAL-002] Medium: 中文硬编码到 durable event payload

`subrun_events.rs:47`:
```rust
"子 Agent 已完成，但没有返回可读总结。".to_string()
```

这作为 fallback 消息写入 `StorageEventPayload::SubRunFinished` 的 durable 事件。durable 事件数据应保持语言无关或至少使用 UI 层可替换的 key，而非直接嵌入面向用户的中文文本。

**Fix**: 使用英文/技术性 fallback（如 `"sub-agent completed without readable summary"`），UI 层负责本地化。

### [QUAL-003] Medium: `wait_for_turn_terminal_snapshot` 无内置超时

`watcher.rs:26-56` 的 `loop` 只在找到 terminal snapshot 时返回。若 turn 因外部原因（session 删除、存储损坏）永远不终结，调用者无限阻塞。测试中使用了外部 `tokio::time::timeout`，但 API 本身没有强制超时。

**Fix**: 考虑在 API 层加入可选的 `timeout` 参数，或在内部加入最大等待轮次后 fallback 到 `try_turn_terminal_snapshot` 一次后返回 error。

---

## ✅ Tests

**Run results**: 1011 passed, 0 failed, 0 ignored (all workspace crates)

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| Medium | `PostLlmDecisionPolicy::decide` 的 `BudgetAllowsContinuation` 分支无独立测试 | `session-runtime/src/turn/post_llm_policy.rs:96-100` |
| Medium | `SessionStateEventSink::emit` 无直接测试（async mutex lock 路径） | `session-runtime/src/state/execution.rs:79-84` |
| Low | `ProjectionRegistry` 无独立测试模块（仅通过 `SessionState` 间接覆盖） | `session-runtime/src/state/projection_registry.rs` |

### [TEST-001] Medium: `PostLlmDecisionPolicy` 预算续写分支缺乏独立断言

`post_llm_policy.rs` 测试覆盖了 `ExecuteTools`、`OutputContinuation`、`diminishing_returns`、`Completed` fallback 四条路径，但 `BudgetAllowsContinuation`（即 `decide_budget_continuation` 返回 `Continue` 的场景）没有专门测试用例。这条路径是 `decide` 函数的最终分支，直接影响 turn 是否继续执行。

**Fix**: 添加测试用例覆盖 `output continuation not needed` + `no diminishing returns` + `budget allows` 场景。

---

## 🏗️ Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| Low | `AgentPromptSubmission` 公开导出但包含 kernel 内部类型 | `session-runtime/src/turn/submit.rs:62-74`, `session-runtime/src/lib.rs:54` |

Crate 边界检查: **PASS** ✅

三层分离合规性:
- 事件溯源层（ProjectionRegistry, projector）: 纯函数/确定性 ✅
- 运行时状态层（TurnRuntimeState, CancelToken）: 内部不暴露 ✅
- 外部接口层（SessionRuntime 公共 API, ports）: 纯数据 DTO ✅

`WorkflowOrchestrator` 在 `application` 中正确消费 `core` 定义的 `WorkflowDef` 类型。

`SessionRecoveryCheckpoint` 在 `core/ports.rs` 中使用 `#[serde(flatten)]` + `LegacySessionRecoveryProjection` 处理旧格式迁移——虽然是向后兼容 hack，但项目声明不维护向后兼容，可接受为一次性迁移。

### [ARCH-001] Low: `AgentPromptSubmission` 公开导出包含运行时关联类型

`submit.rs:62-74` 的 `AgentPromptSubmission` 包含 `ApprovalPending<CapabilityCall>`、`CapabilityRouter` 等 kernel 关联类型，通过 `lib.rs:54` 公开导出。虽然 application 层通过 `AppAgentPromptSubmission` + `.into()` 转换来隔离，但 session-runtime 的公共 API 仍然暴露了 kernel 的具体类型。

**Fix**: 可考虑将 `AgentPromptSubmission` 改为 `pub(crate)` 或在 application port 层完全重新定义，避免 session-runtime 的公共 API 泄漏 kernel 类型。优先级低，当前通过 port 隔离已足够。

---

## 🚨 Must Fix Before Merge

*(Critical/High only. If empty, diff is clear to merge.)*

1. **[QUAL-001]** `set_compacting(true)` 无 panic-safe 保护 — `crates/session-runtime/src/turn/finalize.rs:93-105` + `crates/session-runtime/src/command/mod.rs:163-165`
   - Impact: panic 导致 compacting 标志永久卡死，session 无法再执行 manual compact
   - Fix: 用 RAII guard 或 `scopeguard` 确保 `set_compacting(false)` 始终执行

---

## 📎 Pre-Existing Issues (not blocking)
- `normalize_session_id` 仅做 trim + prefix strip，不做路径遍历字符过滤（当前安全因为仅内部调用）

## 🤔 Low-Confidence Observations
- `WorkflowStateService::persist` 使用 `fs::write` 而非原子写入（write-then-rename），崩溃时可能损坏 state 文件。但 `load_recovering` 能优雅降级，实际风险有限。
- `subrun_finished_event` 生成 `idempotency_key` 时使用 `format!("subrun-finished:{}:{}", ...)` — 如果 `sub_run_id` 包含特殊字符，key 格式可能不符合消费端预期。当前 sub_run_id 由内部生成，风险极低。
