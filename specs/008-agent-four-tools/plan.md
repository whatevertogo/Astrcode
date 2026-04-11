# Implementation Plan: Astrcode Agent 协作四工具重构

**Branch**: `008-agent-four-tools` | **Date**: 2026-04-11 | **Spec**: [spec.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/spec.md)  
**Input**: Feature specification from `D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/spec.md`

## Summary

将 Astrcode 的公开协作面收敛为 `spawn`、`send`、`observe`、`close` 四个工具，并把协作语义从“单轮完成即子 Agent 终态”改成“持久 Agent 实例 + Idle 生命周期”。实现上不新建独立 router/WAL，而是基于现有 `agent_control`、session event log 和动态 `PromptDeclaration` 做三项核心升级：把 root agent 注册进同一棵控制树、把 `AgentStatus` 拆成生命周期与最近一轮结果、把 mailbox durable 化并显式采用 `at-least-once` + `delivery_id` 语义。新 child agent 统一写为 `IndependentSession`，旧协作工具和 `SharedSession` 新写路径一并清理。

## Technical Context

**Language/Version**: Rust 2021，前端与桌面壳为 TypeScript + React + Tauri 2，但本特性主改 Rust workspace  
**Primary Dependencies**: `tokio`、`serde`、`serde_json`、`uuid`、`chrono`、`tracing`、`axum`，以及 `crates/core`、`crates/runtime`、`crates/runtime-agent-control`、`crates/runtime-agent-tool`、`crates/runtime-prompt`、`crates/runtime-session`、`crates/storage`  
**Storage**: 现有 session event log（`storage` + `runtime-session`），仅支持单事件 `append`；live inbox / wake queue 继续由 `runtime-agent-control` 持有  
**Testing**: `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`，辅以旧工具名搜索、mailbox replay/observe 权限/close 竞态专项测试  
**Target Platform**: Rust workspace 驱动的桌面应用运行时、HTTP/SSE 服务与前端调用层  
**Project Type**: 多 crate 桌面应用运行时与服务端协作子系统重构  
**Performance Goals**: 父子消息在下一个可用 turn 被消费；无静默消息丢失；observe 快照可在不阻塞的前提下返回稳定状态；正在运行的 agent 当前轮 100% 只消费 turn-start batch  
**Constraints**: 不做向后兼容公开入口；存储层没有事务批写；动态 prompt 注入不是 durable transcript；注释与文档必须使用中文；最终实现必须通过仓库级 Rust 校验  
**Scale/Scope**: 影响 `crates/core`、`crates/runtime-agent-control`、`crates/runtime-agent-tool`、`crates/runtime`、`crates/runtime-agent-loop`、`crates/runtime-prompt`、`crates/runtime-session`、`crates/runtime-execution`、`crates/server` 以及 `frontend/src/hooks/useAgent.ts`、`frontend/src/lib/api/sessions.ts`

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Durable Truth First**: 通过。协作消息 durable 真相源定义为 session event log 中新增的 mailbox 事件；`runtime-agent-control` 的 inbox 和 wake queue 只作为 live overlay，不再单独承担历史真相。
- **One Boundary, One Owner**: 通过。`core` 持有协作 DTO、事件和状态枚举；`runtime-agent-control` 持有树结构、live inbox 和 wake queue；`runtime` 持有调度与权限编排；`runtime-prompt`/`runtime-agent-loop` 只负责 mailbox 注入格式；`server`/`frontend` 作为调用方迁移层。
- **Protocol Purity, Projection Fidelity**: 通过。本计划明确区分 durable mailbox 事件、live handle、`observe` 快照 DTO 和 UI 时间线通知，不把 mailbox 状态硬塞进现有对话投影。
- **Ownership Over Storage Mode**: 通过。计划显式把执行 ownership 与 storage mode 拆开，新 child 一律 `IndependentSession`，legacy `SharedSession` 仅保留读取能力，不再作为新写路径。
- **Explicit Migrations, Verifiable Refactors**: 通过。本计划包含旧工具调用方清单、删除顺序、边界迁移策略和具体验证命令，并明确不做公开兼容。
- **Runtime Robustness**: 通过。调度设计显式约束 `snapshot drain`、`Started/Acked` 顺序、`Terminated` 拒收 `send`、`close` 清理 pending wake item，避免模糊状态和 fire-and-forget 协作。
- **Observability & Error Visibility**: 通过。计划要求 `send`、`observe`、`close`、mailbox replay 与 wake 失败全部保留结构化日志和显式错误，不允许静默吞掉非法路由、重复投递或关闭后发送。

**Post-Design Re-check**: 通过。Phase 1 产物已经把 durable 事件、边界 owner、迁移顺序和 prompt 行为写成独立文档，并明确记录 `at-least-once`、`snapshot drain`、`close` subtree 终止等不可回避的行为选择。

## Project Structure

### Documentation (this feature)

```text
specs/008-agent-four-tools/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── agent-collaboration-tool-contract.md
│   └── mailbox-event-contract.md
├── findings.md
├── design-collaboration-runtime.md
├── migration.md
└── tasks.md
```

本特性触发 `findings.md`、`design-*.md`、`migration.md` 三层文档要求，因为它同时：

- 改变 durable 事件格式
- 删除并替换公开 runtime surface
- 触达多个 crate 的边界归属
- 需要清点 server/frontend 等外部调用方

### Source Code (repository root)

```text
crates/
├── core/
│   ├── src/agent/
│   └── src/action.rs
├── runtime-agent-control/
│   └── src/lib.rs
├── runtime-agent-tool/
│   └── src/
├── runtime/
│   └── src/service/execution/
├── runtime-agent-loop/
│   └── src/
├── runtime-prompt/
│   └── src/contributors/
├── runtime-session/
│   └── src/
├── runtime-execution/
│   └── src/policy.rs
├── storage/
│   └── src/
└── server/
    └── src/

frontend/
└── src/
    ├── hooks/useAgent.ts
    └── lib/api/sessions.ts

src-tauri/
└── [桌面宿主，消费 server/runtime 能力，本特性预计无主改动]
```

**Structure Decision**: 以 Rust workspace 内 runtime/control/tool/prompt/session 边界为主战场，前端与 server 只做调用面迁移和命名收敛。mailbox durable 真相源落在 session events，不新增额外持久化子系统。

## Phase 0: Research Summary

> **Phase 映射**: 本文档的 Phase 0（Research）与 Phase 1（Design）对应后续 `tasks.md` 的 Setup / Foundational 阶段；具体 crate 修改、迁移和验证拆分会在 `/speckit.tasks` 中细化。

1. 公开协作面不能只改工具名，必须先把 root agent 注册进控制树，否则 `send(parentId, ...)` 在根层没有真实 owner。
2. 旧 `AgentStatus` 同时承担生命周期和单轮结果，已经无法支撑四工具模型；必须拆成 `AgentLifecycleStatus` 与 `AgentTurnOutcome`。
3. mailbox durable 化不能另起 WAL，因为仓库已经有 session event log；但现有存储只有单事件 `append`，所以 `BatchStarted` 只能定义为 mailbox-wake turn 的第一条 durable 事件，不能假设事务合并。
4. 协作消息必须采用 `at-least-once` + 稳定 `delivery_id`，并把 `snapshot drain` 作为唯一合法的 turn 边界；否则 context 组装和 replay 都不稳定。
5. `AgentStateProjector` 继续负责 phase/turn/output 投影；mailbox pending/active batch 需要独立 projector 或 replay 逻辑，避免对话投影和 mailbox 状态混在一起。
6. 新 child 统一写为 `IndependentSession` 是可行的，因为 `runtime-execution` 已经把它作为默认方向；`SharedSession` 可以只保留 legacy 读取，不再参与新写语义。
7. prompt 与 tool description 不能继续教模型 `wait/sendAgent/closeAgent/resumeAgent/deliverToParent`，必须统一成 `spawn/send/observe/close`，并明确重复 `delivery_id` 是允许的恢复现象。

## Phase 1: Design Plan

### Design Decisions

1. **控制树统一化**: root agent 成为一等控制对象，所有新 child 都挂在真实 parent agent id 下；关系校验继续复用 `agent_control` 的 parent/ancestor 能力。
2. **状态拆层**: `SubRunHandle` 保留现名以控制改动面，但语义升级为持久 agent 句柄，显式持有 `lifecycle_status` 与 `last_turn_outcome`。
3. **mailbox durable 事件**: 新增 `AgentMailboxQueued`、`AgentMailboxBatchStarted`、`AgentMailboxBatchAcked`、`AgentMailboxDiscarded`，并通过 `Queued - Acked - Discarded` 重建 pending。
4. **turn 消费边界**: 每轮开始时做一次 `snapshot drain`，形成固定 `batch_id + delivery_ids`；轮中新增消息 100% 延迟到下一轮。
5. **公开工具合同**: 公开层只保留 `spawn/send/observe/close`；`resume` 仅保留内部预留接口，不注册、不出现在 schema/prompt/few-shot。
6. **observe 增强结果**: `observe` 查询结果同时融合 live handle、现有 `AgentStateProjector` 与 mailbox projector，返回 `activeTask`、`pendingTask`、`pendingMessageCount` 等可直接决策的字段。
7. **通知通道降级而不删除**: `ChildSessionNotification` 继续保留给 UI/timeline，但不再承担父子协作主通道职责。
8. **关闭语义**: `close` 当前只做 subtree terminate，关闭时 durable 丢弃未 acked mailbox、取消运行中 turn、清理 pending wake item；不支持 detach / preserve descendants。

### Migration Order

1. **Core 契约层**: 拆分 `AgentStatus`，新增四工具 DTO、mailbox 事件与 `delivery_id`/`batch_id` 相关模型，保留内部 resume 预留。
2. **控制面地基**: 在 `runtime-agent-control` 注册 root agent，升级 `SubRunHandle`、live inbox 和 wake queue；新 child 写路径固定为 `IndependentSession`。
3. **mailbox durable/replay**: 在 session event log 加入 mailbox 事件，补 `MailboxProjector` 或等价 replay 能力，并定义 Started/Acked/Discarded 顺序。
4. **runtime 调度**: 用新 `send/observe/close` 替换旧 send/wait/close/resume/deliver 逻辑，接入 snapshot drain、Terminated 拒收与 subtree close。
5. **tool/prompt/loop**: 替换 `runtime-agent-tool` 注册与 schema，重写 `workflow_examples`、spawn 提示和 mailbox 注入声明。
6. **调用方迁移**: 更新 `crates/server`、`frontend/src/hooks/useAgent.ts`、`frontend/src/lib/api/sessions.ts` 以及相关测试，移除旧工具名与旧公开面依赖。
7. **删除旧面并收尾**: 删除旧 DTO/result 分支、旧工具实现、旧 prompt 文案和仅服务旧协作模型的测试。

### Validation Strategy

- 旧工具清理搜索：  
  `rg -n "waitAgent|sendAgent|closeAgent|deliverToParent|resumeAgent" D:/GitObjectsOwn/Astrcode/crates D:/GitObjectsOwn/Astrcode/frontend -g '*.rs' -g '*.ts' -g '*.tsx'`
- 仓库级 Rust 校验：  
  `cargo fmt --all --check`  
  `cargo clippy --all-targets --all-features -- -D warnings`  
  `cargo test`
- 前端调用层校验：  
  `cd frontend && npm run typecheck`
- 核心语义测试：  
  `cargo test -p runtime-agent-control`  
  `cargo test -p runtime-agent-tool`  
  `cargo test -p runtime --lib collaboration`  
  `cargo test -p runtime-agent-loop`
- 关键行为验证：
  - 子 agent 单轮完成后回到 `Idle`
  - 父发 `Idle` child 触发下一轮，父发 `Running` child 只排队
  - 子发父触发父 wake
  - `Started` 后 crash、`Acked` 前重放相同 `delivery_id`
  - `observe` 非直接父拒绝
  - `close` 清理 subtree pending wake item 与 pending mailbox
- 调用面验证：
  - 前端 `useAgent.ts` / `sessions.ts` 不再引用旧工具名
  - prompt/few-shot/tool description 中只剩 `spawn/send/observe/close`
  - `ChildSessionNotification` 仍可驱动 UI/timeline，但不再承载父子 mailbox 主通道
  - 锁、wake queue、spawn handle 路径完成 robustness 审计：无持锁跨 await、无 fire-and-forget 句柄泄漏、无 panic 式锁恢复路径

## Complexity Tracking

无宪法违规需要豁免，但以下复杂度被显式接受并文档化：

| 复杂度点 | 为什么需要 | 放弃的更简单方案 |
|----------|------------|------------------|
| `at-least-once` mailbox 语义 | 当前存储层没有事务批写，也没有 `TurnStarted`，无法低成本获得 exactly-once | 注入即 ack 的 at-most-once 会引入静默消息丢失 |
| 额外 mailbox projector / replay 逻辑 | 现有 `AgentStateProjector` 不适合承载 pending/active batch 语义 | 把 mailbox 状态硬塞进现有对话投影会污染职责边界 |
| root agent 注册进控制树 | 四工具要支持 root<->child 对称协作，必须有统一 parent/child 控制关系 | 保持 root 仅存在于 `ExecutionAccepted` 会让 `send(parentId)` 在根层没有真实 owner |
