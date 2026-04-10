# Implementation Plan: 删除死代码与冗余契约收口

**Branch**: `006-prune-dead-code` | **Date**: `2026-04-10` | **Spec**: [spec.md](D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/spec.md)
**Input**: Feature specification from `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/spec.md`

## Summary

本次计划把“删除死代码”收口成一次跨 `frontend -> protocol -> server -> runtime -> core` 的正式支持面清扫。
实施策略不是给旧语义再包一层 adapter，而是先确认当前主线真正依赖什么，再把 subrun 领域重复状态、
重复 receipt、重复 descriptor、错误 owner boundary、重复 child open target、弱类型协议状态、分散的 compaction
原因映射和三层重复的 prompt metrics payload 全部收成唯一正式表达，最后同步删除 legacy downgrade 分支、
旧 cancel route、live 文档宣传和只为旧入口续命的测试。

## Technical Context

**Language/Version**: Rust workspace（Edition 2021）+ TypeScript 5 / React 18 / Vite 5  
**Primary Dependencies**: Tokio / Axum / Tower / Serde / Tracing；React / Vitest / ESLint / Prettier；Tauri 2 壳层  
**Storage**: 本地文件系统会话仓库与 event log（`crates/storage`），子会话 durable 节点与通知基于 session history / JSONL 投影  
**Testing**: `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`、`frontend` 的 `npm run typecheck && npm run lint && npm run format:check && npm run test`  
**Target Platform**: 本地桌面应用（`src-tauri`）+ Axum HTTP/SSE server + React 单页前端  
**Project Type**: 多 crate Rust workspace + desktop frontend/backend monorepo  
**Performance Goals**: 每个保留用户动作只保留一条正式路径；不引入新的 projection/fallback 层；当前会话浏览、focused subrun 浏览、child session 直开和消息提交流程保持可用  
**Constraints**: 不要求向后兼容；`protocol` 与 `core` 不得直接依赖；`runtime-*` 只依赖 `core`；删除 public surface 时必须同步更新 docs/tests；主线能力不能留下功能缺口  
**Scale/Scope**: 涉及 `crates/core`、`crates/runtime`、`crates/runtime-execution`、`crates/runtime-agent-control`、`crates/runtime-agent-loop`、`crates/protocol`、`crates/server`、`frontend`、`docs/spec` 和当前 feature 规格文档的跨层收口

## Constitution Check

*GATE: 已通过。Phase 1 设计完成后已复检。*

### Pre-Phase 0

- **Durable Truth First**: PASS  
  计划把 lineage、状态、child navigation 和 summary 明确分为 durable truth 与 read model。旧 downgrade 视图会被删除，而不是继续作为“读侧补救层”保活；child open target 收口为 `child_ref.open_session_id`，外层重复字段删除。
- **One Boundary, One Owner**: PASS  
  `core` 持有唯一 canonical 模型，`runtime` 只负责编排与 live control，`server` 只做协议投影，`frontend` 只保留当前 UI 消费的 read model。重复 owner 会被删除或迁移，compaction 原因到 durable trigger 的映射也将集中到单一 owner。
- **Protocol Purity, Projection Fidelity**: PASS  
  协议层仍保持 DTO-only；状态、child navigation 和 prompt receipt 将通过 server mapper 从 runtime/core 事实投影，不在 `protocol` 中重新发明策略逻辑。child/subrun 状态将使用 DTO 枚举而非字符串；`PromptMetrics` 改为共享 payload 投影。
- **Ownership Over Storage Mode**: PASS  
  `parent_turn_id`、`sub_run_id`、`child_session_id` 等 ownership 事实将从 `SubRunHandle` / durable child node 显式建模，不再通过 descriptor 缺失、storage mode 或 legacy source 反推。
- **Explicit Migrations, Verifiable Refactors**: PASS  
  `migration.md` 明确列出 caller inventory、迁移顺序和验证矩阵；活跃入口（如 cancel）会先切换再删除；新增的协议状态枚举、open target 去重和 prompt metrics payload 收口会同步更新 contract 与 quickstart 验证。
- **Runtime Robustness**: PASS  
  方案没有新增 fire-and-forget 或锁跨 await 的设计；相反会删除 descriptorless / downgrade 分支，减少运行时分支复杂度，并收紧重复状态/重复字段的漂移风险。
- **Observability & Error Visibility**: PASS  
  对不再支持的旧输入，方案要求明确失败，不再输出“部分可用”视图或静默空结果；`PromptMetrics` 共享 payload 会保留现有指标可观测性，而不是通过三份字段清单分散维护。

### Post-Design Re-check

- **Durable Truth First**: PASS  
  设计已经把 `AgentStatus`、`ExecutionAccepted`、required `parent_turn_id`、唯一 child open target、共享 `PromptMetricsPayload` 和集中 compaction trigger 映射固定为单一 truth。
- **One Boundary, One Owner**: PASS  
  `launch_subagent` 将迁入 `LiveSubRunControlBoundary`；`ChildAgentRef` 被收口成身份事实加 canonical open target，不再承载 `openable` 等 UI 派生字段；notification 外层不再重复存 `open_session_id`。
- **Protocol Purity, Projection Fidelity**: PASS  
  dead route 删除后，protocol 不再暴露 parent summary/view projection；保留的协议只描述当前主线和明确失败边界，并以强类型 DTO 枚举表达 child/subrun 状态。
- **Ownership Over Storage Mode**: PASS  
  `SubRunDescriptor` 删除后，ownership 只通过 `SubRunHandle` / durable child node 表达，`legacyDurable` 退出主线。
- **Explicit Migrations, Verifiable Refactors**: PASS  
  设计文档已把“核心模型收口”“orphan surface 删除”“cancel cutover”“legacy failure 收口”“protocol 状态枚举替换”“PromptMetrics payload 去重”和“compaction 映射集中化”拆成可验证阶段。
- **Runtime Robustness**: PASS  
  通过减少重复状态、descriptor downgrade、重复 open target 和分散映射，可降低条件分支与错误投影路径。
- **Observability & Error Visibility**: PASS  
  旧输入统一转为显式失败；注释合同明确要求调用方不要绕过 `model_content()` 等关键入口；共享指标 payload 不削弱日志与指标边界。

## Project Structure

### Documentation (this feature)

```text
specs/006-prune-dead-code/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── findings.md
├── design-surface-pruning.md
├── design-legacy-cutover.md
├── design-subrun-contract-consolidation.md
├── migration.md
└── contracts/
    ├── retained-surface-contract.md
    ├── summary-and-navigation-contract.md
    ├── legacy-failure-and-control-cutover.md
    └── subrun-canonical-contract.md
```

### Source Code (repository root)

```text
crates/
├── core/
│   └── src/
│       ├── action.rs
│       ├── agent/mod.rs
│       ├── event/
│       ├── hook.rs
│       └── runtime/traits.rs
├── runtime/
│   └── src/service/
│       ├── execution/
│       │   ├── context.rs
│       │   ├── mod.rs
│       │   ├── root.rs
│       │   ├── status.rs
│       │   └── subagent.rs
│       ├── observability.rs
│       └── service_contract.rs
├── runtime-execution/
│   └── src/subrun.rs
├── runtime-session/
│   └── src/session_state.rs
├── runtime-agent-control/
│   └── src/lib.rs
├── runtime-agent-loop/
│   └── src/
│       ├── agent_loop.rs
│       └── compaction_runtime.rs
├── protocol/
│   └── src/http/
│       ├── agent.rs
│       ├── event.rs
│       ├── session.rs
│       └── mod.rs
├── server/
│   └── src/
│       ├── http/
│       │   ├── mapper.rs
│       │   └── routes/
│       │       ├── mod.rs
│       │       ├── runtime.rs
│       │       ├── tools.rs
│       │       └── sessions/
│       │           ├── mutation.rs
│       │           └── query.rs
│       └── tests/
└── storage/
    └── src/session/

frontend/
└── src/
    ├── App.tsx
    ├── hooks/useAgent.ts
    ├── lib/
    │   ├── agentEvent.ts
    │   ├── api/sessions.ts
    │   ├── sessionHistory.test.ts
    │   ├── subRunView.test.ts
    │   └── subRunView.ts
    └── types.ts

docs/
└── spec/
```

**Structure Decision**: 这是一个 Rust 多边界运行时 + React/Tauri 前端的桌面应用。实现会遵守现有边界，把 canonical 模型收回 `core`，把 route/DTO 投影保留在 `server` + `protocol`，把 UI 专属派生逻辑限制在 `frontend`，不在 `runtime` 或 `core` 里继续承载 view-only 字段或重复 payload 定义。

## Planned Workstreams

### 1. 正式支持面清点与删除

- 删除无人消费的 frontend API / projection：
  - `loadParentChildSummaryList`
  - `loadChildSessionView`
  - `buildParentSummaryProjection`
  - `ParentSummaryProjection`
  - `ChildSummaryCard`
- 删除无人消费的 public HTTP surface：
  - `/api/sessions/{id}/children/summary`
  - `/api/sessions/{id}/children/{child_session_id}/view`
  - `/api/v1/agents*`
  - `/api/v1/tools*`
  - `/api/runtime/plugins*`
  - `/api/config/reload`

### 2. Subrun canonical contract 收口

- `SubRunOutcome` 并入 `AgentStatus`，保留 `TokenExceeded` 正式终态
- 删除 `SubRunDescriptor`，让 `SubRunHandle.parent_turn_id` 成为必填
- 把 `PromptAccepted` / `RootExecutionAccepted` 及 runtime duplicate 收口为 `ExecutionAccepted`
- 为 `AgentEventContext` 增加 `From<&SubRunHandle>`，减少手工拼装
- 把 `launch_subagent` 迁入 `LiveSubRunControlBoundary`
- 把 `ChildAgentRef` 收口成 identity / lineage / status / `open_session_id` 的正式 child reference，删除 `openable`
- 删除 `ChildSessionNotification` 与 protocol DTO 外层重复 `open_session_id`

### 3. Protocol 与 event payload 去重

- 为 child/subrun 相关状态补齐 `AgentStatusDto` 或等价强类型 DTO 枚举
- server mapper 与 frontend reader 改为消费枚举状态，而不是字符串匹配
- 为 `PromptMetrics` 提取共享 payload，避免 storage event / agent event / protocol event 三层逐字段复制
- 校验 `/history` 与 `/events` 在共享 payload 和强类型状态下仍保持一致 envelope 语义

### 4. Compaction 与 child navigation cutover

- `Reactive` 仅保留为 runtime / hook 内部 compaction reason
- durable `CompactTrigger` 仍保持正式 trigger 集合，并提供唯一集中映射
- child navigation 只依赖 canonical `child_ref.open_session_id` 与 durable child fact，不再依赖 summary projection API 或 duplicated open flags
- 当前 UI 的 cancel 主线切到 `closeAgent`

### 5. 明确失败与文档测试同步

- `legacyDurable`、descriptorless downgrade、旧共享历史半兼容视图统一改为明确失败
- 删除只为旧入口存在感服务的测试
- 更新 live 文档、开放项和 quickstart，让仓库只描述保留能力

## Complexity Tracking

本次计划无已知宪法违例，不需要保留额外复杂度豁免。
