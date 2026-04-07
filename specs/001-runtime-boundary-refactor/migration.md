# Migration Plan: Runtime Boundary Refactor

## Why Three-Layer Docs Are Mandatory Here

本 feature 同时满足以下强制触发条件：

- 变更 durable event 字段
- 变更公共 runtime / protocol surface
- 变更边界依赖方向
- 删除有外部调用方的 façade

因此迁移必须按 findings / design / migration 分层执行，不能混成一份“边改边说”的总文档。

## Caller Inventory

### A. 当前 legacy façade 的直接与间接调用方

| Surface | Current implementation | Current callers |
|---------|------------------------|-----------------|
| `RuntimeService::load_session_history()` | `service/session/load.rs` -> `session_service.rs` | `crates/server/src/http/routes/sessions/query.rs`、`crates/server/src/http/routes/sessions/stream.rs` |
| `RuntimeService::replay()` | `service/replay.rs` -> `execution_service.rs` | `crates/server/src/http/routes/sessions/stream.rs` |
| `RuntimeService::submit_prompt()` | `service/turn/submit.rs` -> `execution_service.rs` | `crates/server/src/http/routes/sessions/mutation.rs` |
| `RuntimeService::interrupt()` | `service/turn/submit.rs` -> `execution_service.rs` | `crates/server/src/http/routes/sessions/mutation.rs`、`crates/runtime/src/service/session_service.rs` |
| `session_service()` helper | `service/mod.rs` | `crates/runtime/src/service/session/create.rs`、`delete.rs`、`load.rs` |
| `execution_service()` helper | `service/mod.rs` | `crates/runtime/src/service/replay.rs`、`crates/runtime/src/service/turn/submit.rs` |

### B. 当前已经在用的新 handle

| Surface | Current callers |
|---------|-----------------|
| `agent_execution_service()` | `crates/server/src/http/routes/agents.rs`、`crates/runtime/src/service/execution/mod.rs`、`crates/runtime/src/runtime_governance.rs` |
| `tool_execution_service()` | `crates/server/src/http/routes/tools.rs` |

### C. 与 working-dir resolver 直接相关的调用点

| Concern | Current callers |
|---------|-----------------|
| root execute 请求 working dir | `crates/server/src/http/routes/agents.rs` |
| runtime bootstrap profile load | `crates/runtime/src/bootstrap.rs` |
| agent watch paths | `crates/runtime/src/service/watch_ops.rs` |

## Stage Plan

## M1. Protocol Foundation

**Goal**  
先把 durable lifecycle event 的 lineage 与 trigger 关系补齐。

**Files / crates**

- `crates/core/src/agent/mod.rs`
- `crates/core/src/event/types.rs`
- `crates/core/src/event/domain.rs`
- `crates/core/src/event/translate.rs`
- `crates/runtime/src/service/execution/subagent.rs`
- `crates/runtime-execution/src/subrun.rs`
- `crates/protocol/src/http/event.rs`
- `crates/protocol/src/http/agent.rs`
- `crates/server/src/http/mapper.rs`
- `frontend/src/types.ts`
- `frontend/src/lib/agentEvent.ts`

**What changes now**

- 新增 `SubRunDescriptor`
- 生命周期事件写入 `descriptor` + `tool_call_id`
- status API 增加 `descriptor` / `toolCallId` / `source`
- legacy 历史反序列化与 `legacyDurable` 降级

**What does not change yet**

- 不删除 façade
- 不重做五边界 compile-time 依赖
- server filter / frontend tree 还可以先保持原实现，但要能消费新 payload

**Validation**

```powershell
cargo test -p astrcode-protocol subrun_event_serialization
cargo test -p astrcode-runtime-execution subrun
Set-Location frontend
npm run test -- agentEvent
```

## M2. Query And Projection Convergence

**Goal**  
让 `/history`、`/events` 和 frontend subrun tree 从同一份 durable lineage index 工作。

**Files / crates**

- `crates/runtime-execution/src/subrun.rs` 或新 lineage index 模块
- `crates/server/src/http/routes/sessions/filter.rs`
- `crates/server/src/http/routes/sessions/query.rs`
- `crates/server/src/http/routes/sessions/stream.rs`
- `frontend/src/lib/subRunView.ts`
- `frontend/src/lib/sessionHistory.ts`（若需要）

**What changes now**

- server filter 去掉 `parent_turn_id -> turn_owner` 亲缘推断
- frontend 去掉 `parentTurnId -> turnOwnerMap` 亲缘推断
- `scope=directChildren` / `scope=subtree` 对 legacy lineage gap 返回显式错误

**Deletion preconditions**

- lineage index 已能从 lifecycle event 独立构树
- `/history` 与 `/events` 的 filtered replay 共享同一实现

**Validation**

```powershell
cargo test -p astrcode-server session_history_endpoint_filters_subrun_scope_and_cursor
cargo test -p astrcode-server session_events_contract_rejects_scope_without_subrun_id
Set-Location frontend
npm run test -- subRunView
```

## M3. Boundary Extraction

**Goal**  
把执行编排从 `runtime-session` 和 legacy façade 中抽离到 `runtime-execution`，恢复单向依赖。

**Files / crates**

- `crates/core`：补共享 trait
- `crates/runtime-session`：移除 `runtime-agent-loop` / `runtime-agent-control` 依赖
- `crates/runtime-execution`
- `crates/runtime-agent-loop`
- `crates/runtime-agent-control`
- `crates/runtime/src/service/execution/*`

**What changes now**

- `runtime-session` 只保留 session/turn ledger 与 durable 写入
- `runtime-execution` 接管 submit/interrupt/root execute/subrun launch/status orchestration
- `runtime-agent-loop` 与 `runtime-agent-control` 通过 `core` trait 被注入

**Deletion preconditions**

- `crates/runtime-session/Cargo.toml` 不再依赖 `runtime-agent-loop` / `runtime-agent-control`
- `turn_runtime.rs` 不再直接导入 `AgentControl` / `AgentLoop`

**Validation**

```powershell
cargo check --workspace
cargo test -p astrcode-runtime-session
cargo test -p astrcode-runtime-execution
```

## M4. Caller Migration

**Goal**  
让 server 与 runtime 内部调用方全部迁到唯一 owner surface。

**New target surfaces**

- `RuntimeService::sessions()`
- `RuntimeService::execution()`
- `RuntimeService::tools()`

**Move these callers**

- `crates/server/src/http/routes/sessions/query.rs` -> `sessions().history()`
- `crates/server/src/http/routes/sessions/stream.rs` -> `sessions().replay()`
- `crates/server/src/http/routes/sessions/mutation.rs` -> `execution().submit_prompt()` / `execution().interrupt_session()`
- `crates/server/src/http/routes/agents.rs` -> `execution()`
- `crates/runtime/src/service/execution/mod.rs` 的 `DeferredSubAgentExecutor` -> `execution()`
- `crates/runtime/src/runtime_governance.rs` -> `execution()`

**Deletion preconditions**

- `session_service()` 没有剩余 call site
- `execution_service()` 没有剩余 call site
- `RuntimeService::{load_session_history,replay,submit_prompt,interrupt}` 不再被 server 直接调用

**Validation**

```powershell
cargo test -p astrcode-server
cargo check --workspace
```

## M5. Delete Legacy Façades And Finish Resolver Migration

**Goal**  
删除 legacy façade，并把 working-dir resolver 绑定到 execution context，而不是进程 cwd。

**Delete**

- `crates/runtime/src/service/session_service.rs`
- `crates/runtime/src/service/execution_service.rs`
- `crates/runtime/src/service/replay.rs`
- `crates/runtime/src/service/turn/submit.rs`
- `crates/runtime/src/service/session/load.rs` 中纯 façade 转发逻辑

**Resolver completion**

- `bootstrap.rs` 不再用 `agent_loader.load()` 初始化全局 cwd 快照
- `watch_ops.rs` 不再以 `std::env::current_dir()` 为唯一 watch scope
- root execute 必须显式 `workingDir`

**Validation**

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
Set-Location frontend
npm run typecheck
npm run test
npm run lint
npm run build
```

## Merge Strategy

推荐一阶段一 PR，原因如下：

1. M1 可以单独验证 durable 协议是否成立。
2. M2 可以单独验证 server/frontend 是否停止使用 ancestry 启发式。
3. M3 会动 crate 依赖方向，最好和协议改动分开评审。
4. M4/M5 才适合删除 façade；把删除动作放到最后，caller inventory 才稳定。

## Done Criteria

只有同时满足以下条件，整个迁移才算完成：

1. subrun durable lifecycle event 已包含 descriptor + tool_call_id。
2. `/history`、`/events`、status query、frontend subrun tree 都使用同一 lineage 语义。
3. `runtime-session` 不再依赖 `runtime-agent-loop` / `runtime-agent-control`。
4. `session_service.rs` 与 `execution_service.rs` 已删除。
5. root execute 的 agent 解析只由显式 `workingDir` 或 session context 决定。

