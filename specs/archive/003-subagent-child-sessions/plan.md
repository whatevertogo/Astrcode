# Implementation Plan: 子 Agent Child Session 与协作工具重构

**Branch**: `003-subagent-child-sessions` | **Date**: 2026-04-08 | **Spec**: [spec.md](./spec.md)  
**Input**: Feature specification from `/specs/003-subagent-child-sessions/spec.md`

## Summary

本次计划把当前 `spawnAgent + subrun thread tree` 的半成品模型收敛成一套可持续的 child-session 协作底座，分成三条主线同步推进：

1. 把子 agent 提升为 durable 的独立子会话和 agent 所有权节点，父 turn 只负责触发，不再拥有子 agent 生命周期。
2. 把主子协作统一收敛到 tool 契约层，代码推荐放在runtime-agent-tool crates 里面，提供 `spawn / send / wait / close / resume / deliver` 这一组协作能力；runtime 内部用定向 inbox 投递和唤醒实现，而不是递归重放 tool 流。1
3. 把前端从“父会话里混读子执行消息”的 `subRunThreadTree` 模型迁移到“父侧摘要投影 + 子侧完整会话”的双层视图，只显示思考、工具活动和最终回复，不再把原始 JSON 暴露给 UI。

本计划默认接受内部破坏性调整：`spawnAgent` 周边 Rust surface、server 路由返回结构、前端 subrun/thread tree read model 都可以同步升级；不为旧的内部抽象保留长期兼容层。唯一保留的兼容义务是：新实现仍能读取旧的 subrun durable 历史，但必须把其 lineage 或协作能力缺失明确标注为 legacy 降级，而不是继续伪造完整 child-session 语义。

## Technical Context

**Language/Version**: Rust 2021 workspace；TypeScript 5 + React 18  
**Primary Dependencies**: `tokio`, `axum`, `serde`, `serde_json`, `uuid`, `tracing`, `dashmap`；前端使用 `vite` 5、`vitest`、`eslint`、`react-markdown`  
**Storage**: append-only JSONL session event logs（`~/.astrcode/projects/<project>/sessions/<session-id>/session-<session-id>.jsonl`）+ live agent control state + SSE history/events projection  
**Testing**: `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`、`cd frontend && npm run typecheck && npm run test && npm run lint && npm run build`  
**Target Platform**: Tauri 桌面应用 + 本地 HTTP/SSE 服务 + React SPA  
**Project Type**: Rust workspace backend + React frontend + Tauri shell  
**Performance Goals**: 协作工具调用必须快速返回控制权；父视图只渲染 child summary 投影，不能为了展示子 agent 状态去重放完整 child transcript；打开子会话必须直接加载目标 session，而不是从父会话混合消息里再次推断  
**Constraints**: 协作契约统一表现为 tool，但 runtime 内部必须走定向 inbox/mailbox；child session durable truth 不能依赖 live registry 或 UI path；前端默认不展示 raw JSON；不得新增 panic 路径、fire-and-forget 任务或持锁 await  
**Scale/Scope**: 影响 `crates/core`、`crates/runtime-agent-tool`、`crates/runtime-execution`、`crates/runtime-agent-control`、`crates/runtime-session`、`crates/runtime-registry`、`crates/runtime-prompt`、`crates/runtime`、`crates/protocol`、`crates/server`、`frontend/src`

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

本 feature 同时命中 durable 行为变化、公共 runtime/tool surface 扩展、边界 owner 调整这三类高风险项，因此必须使用 findings / design / migration 三层文档，而不是只写一个轻量计划。

### Pre-Phase Gate

- **Durable Truth First**: PASS。计划明确把 child session 节点、协作 inbox envelope、父侧任务通知的 durable 记录作为历史真相；live agent control 与 UI tree 只做运行态 overlay。
- **One Boundary, One Owner**: PASS。计划把 `runtime-agent-tool`、`runtime-execution`、`runtime-agent-control`、`runtime-session`、`runtime-registry`、`server`、`frontend` 的 owner 职责分别写入 `design-collaboration-runtime.md` 和 `design-parent-child-projection.md`。
- **Protocol Purity, Projection Fidelity**: PASS。计划要求 tool 契约、server DTO 与 history/events 投影分别定义，不把 runtime 默认值或 UI 启发式塞回 `protocol`。
- **Ownership Over Storage Mode**: PASS。计划把 parent/child 所有权、agent 引用、fork lineage 与物理存储路径分离；是否使用 parent-local 子目录只能是实现优化，不能成为 ownership 真相。
- **Explicit Migrations, Verifiable Refactors**: PASS。计划显式生成 `findings.md`、两份 design 文档、`migration.md` 和 `contracts/`，并提供完整验证命令与定向回归矩阵。
- **Runtime Robustness**: PASS。计划显式审查 `tokio::spawn`、handle 管理、取消传播、恢复去重和 inbox 消费幂等，避免新协作链路重新引入 fire-and-forget 或双消费。
- **Observability & Error Visibility**: PASS。计划要求 child session 创建、消息投递、父侧唤醒、关闭传播、legacy 降级都必须有结构化日志和稳定错误码。

### Post-Phase Re-Check

- **Durable Truth First**: PASS。`research.md` 与 `data-model.md` 明确 child session durable truth 由 `ChildSessionNode`、`AgentInboxEnvelope`、`CollaborationNotification` 构成；`design-collaboration-runtime.md` 禁止依赖父会话混合消息推断子会话状态。
- **One Boundary, One Owner**: PASS。`design-collaboration-runtime.md` 把工具契约层、runtime 投递层、session durable truth、父/子视图投影分别归属到不同 crate/层。
- **Protocol Purity, Projection Fidelity**: PASS。`contracts/agent-collaboration-tools.md` 与 `contracts/session-history-and-child-notifications.md` 分别约束 tool surface 与 server/frontend 投影。
- **Ownership Over Storage Mode**: PASS。`research.md` 明确拒绝把“子会话实际嵌套在父目录里”当成领域事实；`migration.md` 以 agent ref + session id + lineage kind 为主键。
- **Explicit Migrations, Verifiable Refactors**: PASS。`migration.md` 含 caller inventory、阶段目标、删除前提与验证矩阵。
- **Runtime Robustness**: PASS。`data-model.md` 和 `design-collaboration-runtime.md` 定义了幂等投递、单次消费、受控唤醒与 handle 生命周期。
- **Observability & Error Visibility**: PASS。所有关键协作行为都有日志与错误投影要求；父视图默认不再吞 raw JSON 或内部异常串。

## Project Structure

### Documentation (this feature)

```text
specs/003-subagent-child-sessions/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── findings.md
├── design-collaboration-runtime.md
├── design-parent-child-projection.md
├── migration.md
├── contracts/
│   ├── agent-collaboration-tools.md
│   └── session-history-and-child-notifications.md
└── tasks.md
```

使用三层文档的原因：

- 会变更 child session / subrun 的 durable 语义与读模型
- 会扩展模型可调用的协作工具 surface
- 会调整 runtime 内部边界 owner 和 registry 主入口
- 会删除或替换现有的 subrun thread tree / mixed-session 读取启发式

### Source Code (repository root)

```text
crates/
├── core/                         # SpawnAgentParams、SubRunHandle、Capability/Tool 契约、未来 child-agent ref DTO
├── runtime-agent-tool/           # spawnAgent 与后续协作工具族的工具适配层
├── runtime-agent-control/        # live child agent registry、cancel token、状态控制
├── runtime-execution/            # child session orchestration、inbox 投递、handoff/result 组装
├── runtime-session/              # session durable truth、event log、session create/load/replay
├── runtime-registry/             # CapabilityRouter / ToolCapabilityInvoker / tool→capability 适配
├── runtime-prompt/               # prompt-facing agent profile summary 与工具索引
├── runtime/                      # 组装内置工具、execution/session surface、bootstrap
├── protocol/                     # HTTP/SSE DTO 与 capability descriptors
├── server/                       # session routes、subrun/child-session status、history/events 投影
└── storage/                      # JSONL 持久化与 session 路径解析

frontend/
└── src/
    ├── lib/api/                  # session / child-session / collaboration API client
    ├── lib/subRunView.ts         # 现有 mixed-session tree，需要迁移
    ├── hooks/useAgent.ts         # SSE 与 session load orchestration
    └── components/Chat/          # 父会话、子会话、breadcrumb 与 child summary UI

src-tauri/                        # 桌面壳
```

**Structure Decision**: 继续使用现有 Rust workspace + React + Tauri 结构，不引入新服务或新持久化系统。重构重点是重新划清协作工具契约、runtime 投递、child session durable truth 和父/子视图投影四层边界，并在现有仓库目录内完成 caller 迁移。

## Complexity Tracking

本计划没有申请宪法例外。复杂度被限制在以下边界内：

- 不引入第二套持久化数据库或独立消息队列
- 不让 runtime 内部为了“看起来像 tool”而递归执行完整 tool 栈
- 不把 parent/child ownership 真相绑到磁盘目录嵌套
- 不为旧的 mixed-session thread tree 长期保留并行兼容层
