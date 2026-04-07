# Implementation Plan: Runtime Boundary Refactor

**Branch**: `[001-runtime-boundary-refactor]` | **Date**: 2026-04-07 | **Spec**: [spec.md](./spec.md)  
**Input**: Feature specification from `/specs/001-runtime-boundary-refactor/spec.md`

## Summary

本次计划把 runtime / session / agent / subrun 重构收敛成两条主线：

1. 先补齐 durable 子执行事实，给 `SubRunStarted` / `SubRunFinished` 增加稳定 lineage 与 trigger 关联，让 `/history`、`/events`、subrun status 都能在 live 状态消失后仍然复原同一份子执行真相。
2. 再重划五个边界的职责与依赖方向，把 `runtime-session`、`runtime-execution`、`runtime-agent-loop`、`runtime-agent-control` 与 `runtime` façade 的 owner 关系明确化，并删除当前重叠的 `session_service.rs` / `execution_service.rs` 双轨 surface。

本计划默认接受内部破坏性调整：后端、前端与 server 同分支同步演进，不为旧的内部 Rust surface 或旧的前端类型保留兼容层。唯一保留的兼容义务是“新代码仍能读取旧 durable 历史，但必须明确暴露其 lineage 不完整，而不是伪造完整结果”。

## Technical Context

**Language/Version**: Rust 2021 workspace；TypeScript 5 + React 18  
**Primary Dependencies**: `tokio`, `axum`, `serde`, `serde_json`, `chrono`, `dashmap`, `notify`, `tauri` 2；前端使用 `vite` 5、`vitest`、`eslint`、`tailwindcss` 4  
**Storage**: append-only JSONL session event logs（`StoredEvent` / `StorageEvent`）+ 内存 live subrun registry  
**Testing**: `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`、`cd frontend && npm run typecheck && npm run test && npm run lint && npm run build`  
**Target Platform**: Tauri 桌面壳 + 本地 HTTP/SSE API + React SPA  
**Project Type**: Rust workspace backend + React frontend + Tauri shell  
**Performance Goals**: 保持 `/history + /events` 的 durable replay 主链路，不引入第二套持久化投影；过滤和 status 查询允许线性扫描历史，但必须共享同一 lineage 语义  
**Constraints**: 只用现有技术栈；内部 surface 可破坏性调整；旧 durable 历史只做读侧降级，不做回填；边界 owner 必须单一；前后端同分支同步升级  
**Scale/Scope**: 影响 `crates/core`、`crates/runtime-session`、`crates/runtime-execution`、`crates/runtime-agent-control`、`crates/runtime-agent-loop`、`crates/runtime-agent-loader`、`crates/runtime`、`crates/protocol`、`crates/server`、`frontend/src`

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

如果任意原则看起来冲突，先检查是否违反了 **Durable Truth First** 或 **One Boundary, One Owner** 的隐含前提，而不是直接做折中。

### Pre-Phase Gate

- **Durable Truth First**: PASS。计划明确要求 durable lifecycle event 成为 subrun lineage 的唯一历史事实源；`research.md`、`data-model.md`、`design-subrun-protocol.md` 都以此为前提。
- **One Boundary, One Owner**: PASS。计划把五个边界及其 owner、依赖方向、删除 surface 写入 `design-execution-boundary.md`。
- **Protocol Purity, Projection Fidelity**: PASS。计划追踪 durable event、domain event、protocol DTO、`/history` + `/events` 过滤语义与 `SubRunStatus` 的对应关系，见 `design-subrun-protocol.md` 与 `contracts/`。
- **Ownership Over Storage Mode**: PASS。计划把 lineage descriptor 与 storage mode 分离，`SharedSession` / `IndependentSession` 只影响事件写入位置，不影响 ownership 语义。
- **Explicit Migrations, Verifiable Refactors**: PASS。计划显式生成 `findings.md`、`design-subrun-protocol.md`、`design-execution-boundary.md`、`migration.md`，并在 `quickstart.md` 中给出验证命令。

### Post-Phase Re-Check

- **Durable Truth First**: PASS。`findings.md` 证实当前 durable 缺口，`design-subrun-protocol.md` 用 descriptor + trigger link 补齐 durable 真相，并定义 legacy 历史降级。
- **One Boundary, One Owner**: PASS。`design-execution-boundary.md` 指定最终 owner 与禁止承担的职责；`migration.md` 给出 façade 删除顺序。
- **Protocol Purity, Projection Fidelity**: PASS。`contracts/session-history-and-events.md` 与 `contracts/execution-status-and-agent-resolution.md` 约束了 `/history`、`/events`、status API 的统一 lineage 语义。
- **Ownership Over Storage Mode**: PASS。`data-model.md` 将 ownership record 与 storage mode、child session id 分开建模。
- **Explicit Migrations, Verifiable Refactors**: PASS。`migration.md` 包含 caller inventory、阶段目标、删除前提与验证矩阵；没有未解释的兼容层。

## Project Structure

### Documentation (this feature)

```text
specs/001-runtime-boundary-refactor/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── findings.md
├── design-subrun-protocol.md
├── design-execution-boundary.md
├── migration.md
├── contracts/
│   ├── session-history-and-events.md
│   └── execution-status-and-agent-resolution.md
└── tasks.md
```

本 feature 命中三层文档强制触发条件中的全部关键项：

- durable 事件格式会变更
- 公共 runtime / server surface 会变更
- 边界依赖方向会变更
- 现有 façade 与对外可见入口会被删除或替换

### Source Code (repository root)

```text
crates/
├── core/                    # 核心领域类型、StorageEvent、AgentEventContext、trait 契约
├── storage/                 # durable event log 持久化
├── runtime-session/         # session / turn 生命周期与 durable 写入
├── runtime-execution/       # 执行装配、subrun durable 解析、查询策略
├── runtime-agent-control/   # live subrun registry / cancel / depth / concurrency
├── runtime-agent-loop/      # 单次 LLM/tool 主循环
├── runtime-agent-loader/    # working-dir 相关 agent profile 解析与 watch 路径
├── runtime/                 # RuntimeService 门面与装配
├── protocol/                # HTTP / SSE DTO
└── server/                  # Axum routes、HTTP/SSE 投影与过滤

frontend/
└── src/                     # API client、session history、subrun view、store

src-tauri/                   # 桌面壳
docs/architecture/           # 总体架构文档
```

**Structure Decision**: 继续使用现有 Rust workspace + React + Tauri 结构，不引入新语言或新服务。重构重点放在 crate 边界、trait 契约、协议 DTO 与 server/frontend 的消费语义，而不是重新组织仓库目录。

## Complexity Tracking

本计划没有申请宪法例外。复杂度被限制在：

- 只对当前 runtime / session / agent / subrun 主链路动刀
- 不引入第二套 durable schema 或 shadow projection
- 不为了旧 API 保留长期 alias / adapter
- 只对旧 durable 历史保留“读侧降级”而非“双写 + 回填 + 迁移脚本”

