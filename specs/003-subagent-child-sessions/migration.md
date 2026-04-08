# Migration Plan: 子 Agent Child Session 与协作工具重构

## Why Three-Layer Docs Are Mandatory Here

本 feature 同时命中以下强制触发条件：

- child session / subrun 的 durable 语义会变化
- 模型可调用的 runtime/tool surface 会扩展
- runtime-registry / execution / session / frontend 的边界 owner 会调整
- 现有 mixed-session `SubRunThreadTree` 及部分 subrun-only 控制面会被删除或替换

因此必须按 findings / design / migration 分层执行。

## Caller Inventory

### A. 现有 `spawnAgent` tool 直接调用链

| Surface | Current implementation | Current callers |
|---------|------------------------|-----------------|
| `spawnAgent` tool schema / execute | `crates/runtime-agent-tool/src/spawn_tool.rs` | builtin capability 注册、tool tests、prompt 工具定义 |
| child profile summary | `crates/runtime-prompt/src/contributors/agent_profile_summary.rs` | prompt 组装 |
| subagent executor binding | `crates/runtime/src/service/execution/context.rs` + `runtime_surface_assembler.rs` | runtime bootstrap / builtin capability 组装 |

### B. 当前 subrun status / cancel surface

| Surface | Current callers |
|---------|-----------------|
| `GET /api/v1/sessions/{id}/subruns/{sub_run_id}` | server tests、frontend 子执行状态加载 |
| `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel` | `frontend/src/lib/api/sessions.ts::cancelSubRun()`、`frontend/src/hooks/useAgent.ts`、`App.tsx` |
| `RuntimeService::execution().cancel_subrun()` | server route、runtime tests |

### C. 当前前端 child 浏览模型

| Surface | Current callers |
|---------|-----------------|
| `frontend/src/lib/subRunView.ts::buildSubRunThreadTree()` | `App.tsx`、`subRunView.test.ts` |
| `loadSession(sessionId, filter)` | `useAgent.ts`、`App.tsx` |
| `activeSubRunPath` / breadcrumb | store reducer、App、Chat components |

### D. 当前 registry / tool dual-track

| Surface | Current callers |
|---------|-----------------|
| `CapabilityRouter` | runtime bootstrap、runtime-agent-loop、plugin supervisor |
| `ToolRegistry` | runtime tests、runtime-agent-loop tests、execution tests |
| `capability_context_from_tool_context()` | `runtime-registry::router`、tool 执行桥接 |

## Stage Plan

## M1. Durable Child Session Foundation

**Goal**  
先把 child session 节点、ownership 和 notification durable 真相补齐。

**Files / crates**

- `crates/core`
- `crates/runtime-session`
- `crates/storage`
- `crates/runtime-execution`
- `crates/protocol`

**What changes now**

- 引入 `ChildSessionNode`、`ChildAgentRef`、`AgentInboxEnvelope`、`CollaborationNotification`
- 现有 `SubRunHandle` / `SubRunResult` 开始对齐到 durable child-agent ref
- child session 创建与 parent/child lineage 进入 durable events

**What does not change yet**

- 不扩协作工具族
- 不重做前端视图
- 不删除现有 subrun status/cancel route

**Validation**

```powershell
cargo test -p astrcode-runtime
cargo test -p astrcode-server
```

## M2. Collaboration Tool Surface + Registry Convergence

**Goal**  
把协作控制面从 `spawn + cancel` 扩成完整工具族，并收敛到 `CapabilityRouter`。

**Files / crates**

- `crates/runtime-agent-tool`
- `crates/runtime-registry`
- `crates/runtime`
- `crates/runtime-prompt`
- `crates/core`

**What changes now**

- 新增 `sendAgent`、`waitAgent`、`closeAgent`、`resumeAgent`、`deliverToParent`
- `runtime-agent-tool` 只保留工具适配和结果映射
- `CapabilityRouter` 成为唯一生产注册中心
- `ToolRegistry` 退化为测试/装配辅助

**Deletion preconditions**

- builtin capability 全部通过 `ToolCapabilityInvoker` 注册
- 生产路径不再直接依赖 `ToolRegistry` 作为执行主抽象

**Validation**

```powershell
cargo test -p astrcode-runtime-agent-tool
cargo test -p astrcode-runtime-registry
cargo test -p astrcode-runtime-agent-loop
```

## M3. Runtime Inbox / Reactivation / Idempotency

**Goal**  
实现 child ↔ parent 的定向投递、单次消费和 parent reactivation。

**Files / crates**

- `crates/runtime-execution`
- `crates/runtime-agent-control`
- `crates/runtime-session`
- `crates/runtime`

**What changes now**

- inbox/mailbox 投递层落地
- child 完成、等待、继续协作都通过 durable envelope + notification 驱动
- parent 在需要时被重新激活
- close propagation 按 ownership tree 叶子优先执行

**Deletion preconditions**

- 任何父子协作都不再依赖直接混入 parent message stream
- 恢复/重试场景下的双消费问题有测试覆盖

**Validation**

```powershell
cargo test -p astrcode-runtime
```

## M4. Server / Frontend Projection Rewrite

**Goal**  
把 server 与 frontend 从 mixed-session subrun tree 迁到 parent summary + child session direct load。

**Files / crates**

- `crates/server`
- `frontend/src/lib/api`
- `frontend/src/lib/subRunView.ts`
- `frontend/src/hooks/useAgent.ts`
- `frontend/src/App.tsx`
- `frontend/src/components/Chat`

**What changes now**

- server 提供 child-session / notification 投影
- frontend 直接加载 child session
- parent summary card 替代 mixed subrun thread block
- raw JSON 从默认 UI 移除

**Deletion preconditions**

- child session 可通过稳定 session id 打开
- parent summary 足够支撑主视图决策

**Validation**

```powershell
cargo test -p astrcode-server
Set-Location frontend
npm run test -- subRunView
npm run test -- childSession
```

## M5. Delete Legacy Mixed-Session Heuristics

**Goal**  
删除 `SubRunThreadTree` 主路径、旧 subrun-only 控制面和隐式 mixed-session 假设。

**Delete / Replace**

- `frontend/src/lib/subRunView.ts` 的主路径 mixed-session 构树逻辑
- 依赖 `activeSubRunPath` 作为唯一 child 浏览模型的主流程
- 仅围绕 subrun status + cancel 的 server/frontend 调用路径

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

建议一阶段一 PR，原因如下：

1. M1 先建立 durable truth，后续争议都会小很多。
2. M2 可以独立评审工具契约与 registry 收口。
3. M3 是最容易引入竞态和重复消费的部分，最好单独评审。
4. M4/M5 则是明显的 server/frontend read model 切换，适合最后做清理。

## Done Criteria

只有同时满足以下条件，本 feature 才算完成：

1. child session 和 parent/child ownership 已成为 durable 真相。
2. 协作工具族完整可用，且模型侧统一通过 tool 调用。
3. runtime 内部使用 inbox/mailbox 实现送达、唤醒、恢复和去重。
4. parent 会话只消费 notification / summary / tool result projection。
5. child 会话可作为独立 session 打开并查看完整 transcript。
6. `CapabilityRouter` 成为唯一生产注册中心，`ToolRegistry` 仅保留测试辅助职责。
7. 当前 mixed-session `SubRunThreadTree` 主路径已退出生产主流程。

## 实施状态（2026-04-09）

所有迁移阶段已完成：

| Stage | 状态 | 关键实现 |
|-------|------|---------|
| M1: Durable Foundation | ✓ 完成 | `ChildSessionNode`、`ChildAgentRef`、`AgentInboxEnvelope`、`CollaborationNotification` 已在 core/protocol/session 落地 |
| M2: Collaboration Surface | ✓ 完成 | 六个协作工具 + `CapabilityRouter` 收口 + `ToolRegistry` 降级为测试辅助 |
| M3: Inbox / Reactivation | ✓ 完成 | `push_inbox`/`wait_for_inbox`、parent reactivation、leaf-first cascade close、direct-parent delivery |
| M4: Server / Frontend Projection | ✓ 完成 | 父摘要列表 + 子会话直开 API、可折叠 SubRunBlock、raw JSON 默认隐藏 |
| M5: Legacy Cleanup | ✓ 完成 | `buildParentSummaryProjection` 独立于 legacy tree、`cancelSubRun` 标注 legacy、SSE 错误上下文增强 |

**Done Criteria 达标情况**：全部 7 项满足。
