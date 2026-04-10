# Implementation Plan: 删除死代码与兼容层收口

**Branch**: `006-prune-dead-code` | **Date**: 2026-04-10 | **Spec**: [spec.md](./spec.md)  
**Input**: Feature specification from `/specs/006-prune-dead-code/spec.md`

## Summary

本次计划把“删除死代码与兼容层”收敛为五条主线：

1. 以当前真实消费者为准，建立唯一支持面清单，明确哪些能力立即删除、哪些必须先迁移后删除、哪些明确保留。
2. 删除前端无人消费的 parent-child summary API / projection，以及后端只剩骨架或只剩测试自证的 HTTP surface。
3. 把当前仍在 UI 主线上使用的 `cancelSubRun` 流程迁移到 `closeAgent` 协作能力，再删除 legacy cancel route。
4. 清除 legacy 读模型、legacy 状态降级语义和只为旧子智能体历史服务的协议/前端分支，但保留清晰失败能力。
5. 同步收口 live 文档、测试和夹具，让仓库只描述“现在真的支持什么”。

本次规划遵循项目“无需向后兼容”的原则：兼容不是默认义务。唯一需要保留的是当前仍在主线产品流程里的能力；它们必须在同一次变更里完成迁移，不允许长期双轨。

## Technical Context

**Language/Version**: Rust 2021 workspace；TypeScript 5 + React 18  
**Primary Dependencies**: `tokio`、`axum`、`serde`、`serde_json`、`tracing`、`uuid`；前端使用 `vite`、`vitest`、`eslint`  
**Storage**: append-only JSONL session event logs + HTTP/SSE projection + 前端内存态会话树/子执行读模型  
**Testing**: `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`、`cd frontend && npm run typecheck && npm run lint && npm run format:check && npm run test`  
**Target Platform**: Tauri 桌面端 + 本地 Rust HTTP/SSE 服务 + React 单页前端  
**Project Type**: Rust workspace backend + React frontend + Tauri shell  
**Performance Goals**: 本次范围内 0 个“只剩测试/文档引用的公开 surface”；0 条并行主线/兼容线双轨入口；保留主线交互无行为回退  
**Constraints**: 不需要向后兼容；删除公共入口前必须先列出调用方和替代入口；注释和文档必须使用中文；不得误删当前活跃的子执行取消、子会话直开、会话浏览、配置读写和消息提交流程  
**Scale/Scope**: 影响 `frontend/src/lib`、`frontend/src/hooks`、`frontend/src/components`、`frontend/src/types.ts`、`crates/server`、`crates/protocol`、`crates/runtime*`、`docs/spec` 以及相关测试/夹具

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

本 feature 同时命中“删除公共 runtime surface”“删除/替换外部调用接口”“收口跨边界兼容语义”三类高风险项，因此必须提供 `findings.md`、`design-*.md` 与 `migration.md` 三层文档，而不是只给轻量计划。

### Pre-Phase Gate

- **Durable Truth First**: PASS。本次不会把新的真相层引入系统；相反，计划要删除那些绕过当前 durable/history 主线、却没有真实消费者的派生表面能力。保留的子会话直开与摘要事实继续以现有 durable 事件为来源。
- **One Boundary, One Owner**: PASS。计划要求把“谁拥有正式支持面”的问题说清楚：前端只保留当前 UI 消费的读模型，`server` 只保留当前公开且仍有调用方的 HTTP 面，`runtime` 不再背 legacy-only 公开语义。
- **Protocol Purity, Projection Fidelity**: PASS。凡是协议、DTO、事件映射和 `/history` + `/events` 投影仍被主线消费的部分会被保留；无人消费的 summary projection、legacy status downgrade 和骨架端点会从协议面收口。
- **Ownership Over Storage Mode**: PASS。涉及 subrun / child session 的清理会把 ownership 与“曾经如何存储/降级显示”分开处理；旧路径若不再支持，将直接失败，而不是继续让存储模式渗透到公开读模型。
- **Explicit Migrations, Verifiable Refactors**: PASS。计划包含 caller inventory、删除顺序、迁移前置条件和完整验证命令，符合“删除公共入口前必须先列出调用方和替代入口”的宪法要求。
- **Runtime Robustness**: PASS。本次不会新增 fire-and-forget 或 panic 风险；迁移 active cancel 流程时会以现有稳定协作能力替代 legacy route，而不是引入旁路控制逻辑。
- **Observability & Error Visibility**: PASS。计划保留清晰失败能力，不会把 old-history 或 removed-surface 相关错误静默吞掉；同时删除只服务于旧兼容展示的冗余分支。

### Post-Phase Re-Check

- **Durable Truth First**: PASS。[research.md](./research.md) 与 [design-legacy-cutover.md](./design-legacy-cutover.md) 已明确：保留子会话直开和摘要事实，但移除基于 legacy downgrade 的伪可用读模型。
- **One Boundary, One Owner**: PASS。[design-surface-pruning.md](./design-surface-pruning.md) 为前端读模型、server HTTP 面、runtime/协议兼容分支给出了单一 owner 与删除边界。
- **Protocol Purity, Projection Fidelity**: PASS。[contracts/retained-surface-contract.md](./contracts/retained-surface-contract.md) 与 [contracts/summary-and-navigation-contract.md](./contracts/summary-and-navigation-contract.md) 收口了保留面；[contracts/legacy-failure-and-control-cutover.md](./contracts/legacy-failure-and-control-cutover.md) 收口了失败与替代路径。
- **Ownership Over Storage Mode**: PASS。计划要求删除 `legacyDurable` 这类公开降级语义，并把当前活跃控制动作迁移到单一协作入口，不再让 storage mode 或 legacy 样本形态决定公开流程。
- **Explicit Migrations, Verifiable Refactors**: PASS。[migration.md](./migration.md) 给出“立即删”“迁移后删”“最后收尾”的顺序与退出条件。
- **Runtime Robustness**: PASS。active cancel 路径只会迁移到现有 `closeAgent` 能力，不引入新的控制平面分叉。
- **Observability & Error Visibility**: PASS。设计明确要求：旧输入必须明确失败；不再以 legacy-only DTO、UI 降级卡片或测试夹具的形式继续维持错觉。

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
├── migration.md
├── contracts/
│   ├── legacy-failure-and-control-cutover.md
│   ├── retained-surface-contract.md
│   └── summary-and-navigation-contract.md
└── tasks.md
```

使用三层文档的原因：

- 本 feature 会删除或替换公共 runtime / HTTP surface
- 本 feature 会改变协议、前端读模型和 live 文档对“正式支持面”的定义
- 本 feature 会删除带有外部调用方的 legacy 入口，需要显式 caller inventory 和 cutover 顺序

### Source Code (repository root)

```text
frontend/
└── src/
    ├── components/Chat/          # 子执行展示、取消入口、子会话直开
    ├── hooks/                    # API orchestration 与当前活跃流程调用点
    ├── lib/
    │   ├── api/                  # sessions/config/models 等 HTTP client
    │   ├── sessionView.ts        # 当前状态驱动导航与过滤
    │   └── subRunView.ts         # 子执行浏览读模型与待删除摘要投影
    ├── store/                    # activeProjectId / activeSessionId / activeSubRunPath
    └── types.ts                  # legacy/public surface 类型收口

crates/
├── protocol/                     # HTTP DTO 与事件 DTO
├── server/                       # 当前对外 HTTP/SSE surface 与测试
├── runtime-agent-tool/           # `closeAgent` 等协作工具
├── runtime-execution/            # legacy status / lineage 分支的公开语义来源
├── runtime-session/              # legacy 历史相关支持/拒绝逻辑
├── runtime/                      # façade 组合与 service surface
└── core/                         # 子执行摘要事实与协作参数

docs/
└── spec/                         # 当前生效规格与开放项，需与清理后支持面对齐
```

**Structure Decision**: 保持现有 Rust workspace + React + Tauri 结构，不引入新服务或新存储。此次工作专注于删减和收口现有 surface：前端只保留当前状态驱动导航和当前 UI 需要的读模型，server 只保留当前主线公开接口，runtime/protocol 删除 legacy-only 暴露语义。

## Complexity Tracking

本计划没有申请宪法例外。复杂度上限如下：

- 不为了“以后可能还会用”保留未收口骨架接口
- 不为了兼容旧样本继续维持 `legacyDurable` 或等价 downgrade 公开语义
- 不新增新的 server/operator surface 来替代被删除的无人消费端点
- 不把当前活跃 cancel 流程迁移成第二条并行入口；只允许切到 `closeAgent` 单一路径

## Phase 0 Research

Phase 0 已收敛并写入 [research.md](./research.md)，核心决策如下：

1. 以“当前真实消费者 + 明确 owner”作为保留或删除 surface 的唯一标准。
2. 删除无人消费的 parent-child summary API / projection，但保留 `SubRunHandoff.summary` 与 `ChildSessionNotification.summary`。
3. 当前 `cancelSubRun` 不是死代码，必须先迁移 UI 到 `closeAgent`，再删除 REST cancel route。
4. `/api/v1/agents`、`/api/v1/tools`、`/api/runtime/plugins`、`/api/config/reload` 这类无人消费的 public surface 应直接删除，而不是继续挂“未来入口”。
5. 旧共享历史不再通过 `legacyDurable` 等 downgrade 公开语义维持“部分可用”；系统改为明确失败。
6. live 文档与测试基线只保留主线支持面，archive 文档承担历史记录职责。

## Phase 1 Design Outputs

- [findings.md](./findings.md): 记录当前代码库里哪些 surface 真死、哪些是假死、哪些仍有活跃调用方。
- [data-model.md](./data-model.md): 给出本次清理所需的支持面审计模型、迁移关系和验证模型。
- [design-surface-pruning.md](./design-surface-pruning.md): 定义保留、立即删除、迁移后删除三类 surface 的判定与边界。
- [design-legacy-cutover.md](./design-legacy-cutover.md): 定义 legacy 公开语义、降级读模型和文档/测试收口策略。
- [migration.md](./migration.md): 记录调用方清单、切换顺序、删除门槛和回归矩阵。
- [contracts/retained-surface-contract.md](./contracts/retained-surface-contract.md): 说明清理后仍受支持的 server/frontend surface。
- [contracts/summary-and-navigation-contract.md](./contracts/summary-and-navigation-contract.md): 定义保留的 summary 事实与 child navigation 合同。
- [contracts/legacy-failure-and-control-cutover.md](./contracts/legacy-failure-and-control-cutover.md): 定义 legacy failure 方式与 cancel -> `closeAgent` 替代路径。
- [quickstart.md](./quickstart.md): 提供删改后回归命令和人工验收场景。

## Phase 2 Implementation Strategy

### Workstream 1: 建立支持面审计基线

- 列出前端 API、前端投影函数、server HTTP route、runtime/protocol public surface 的真实消费者。
- 标记“立即删除”“迁移后删除”“明确保留”。
- 锁定 live 文档与 open-items 中的冲突表述。

### Workstream 2: 删除立即可删的孤儿 surface

- 删除无人消费的 parent-child summary API client、view projection 和配套 server route。
- 删除无人消费的骨架/运营类 HTTP surface。
- 删除只为这些 surface 自证存在的测试与文档。

### Workstream 3: 迁移活跃 cancel 流程

- 把 Chat -> SubRunBlock -> useAgent 当前 cancel 行为切到 `closeAgent` 协作能力。
- 删除 `cancelSubRun` client wrapper、legacy cancel route 以及对应测试/文档。
- 确保当前“取消子会话”按钮行为保持可用且只有一条主线入口。

### Workstream 4: 收口 legacy public semantics

- 删除 `legacyDurable`、shared-session downgrade、legacy-only subtree helper 分支和仅为旧样本展示服务的前端类型。
- 保留明确失败能力，避免把旧数据伪装成部分可用。
- 调整 protocol / server / frontend 使它们不再对 legacy downgrade 公开建模。

### Workstream 5: 同步文档、测试和夹具

- 更新 `docs/spec`、当前有效开放项和接口说明，删掉过时主线表述。
- 改写测试：从“这个旧东西还在”转向“保留流程可用 + 被删 surface 不再存在”。
- 清理夹具和注释，避免未来误导。

## Validation Gate

实现完成后至少通过以下门槛：

1. `rg -n "loadParentChildSummaryList|loadChildSessionView|buildParentSummaryProjection" frontend crates docs`
2. `rg -n "/api/v1/agents|/api/v1/tools|/api/runtime/plugins|/api/config/reload|subruns/.*/cancel" crates/server frontend docs/spec`
3. `cargo fmt --all --check`
4. `cargo clippy --all-targets --all-features -- -D warnings`
5. `cargo test --workspace --exclude astrcode`
6. `cd frontend && npm run typecheck && npm run lint && npm run format:check && npm run test`
7. 手工验证：
   - 当前会话浏览与消息提交正常
   - 当前子执行聚焦正常
   - 当前子会话直开正常
   - 当前“取消子会话”按钮通过新主线入口工作
   - 已删除 route/导出/文档表述不再出现
