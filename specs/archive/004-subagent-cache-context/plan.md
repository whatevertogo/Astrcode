# Implementation Plan: 子智能体会话与缓存边界优化

**Branch**: `004-subagent-cache-context` | **Date**: 2026-04-09 | **Spec**: [spec.md](./spec.md)  
**Input**: Feature specification from `/specs/004-subagent-cache-context/spec.md`

## Summary

本次计划把 004 feature 收敛为四条必须协同推进的主线，并明确采用“无向后兼容包袱”的干净架构策略：

1. 新子智能体统一进入独立子会话 durable 真相，并删除旧共享写入模式的读取、回放与恢复路径。
2. resume 从“重开一个看起来相似的 child”收紧为“基于 child session durable replay 恢复同一子会话的下一次执行”。
3. 父传子的背景从消息拼接迁移到 `PromptDeclaration -> system blocks`，并复用 `runtime-prompt` 的强指纹与共享缓存边界。
4. 父唤醒从 durable `ReactivationPrompt` 消息迁移到运行时信号与一次性交付输入，父历史只保留可追溯的子边界事实。

本次规划不是在旧模型上叠兼容层，而是一次显式 cutover：共享写入 legacy 历史不再属于本 feature 的支持范围，系统遇到此类数据时必须明确返回 `unsupported` 或 `upgrade required`。

## Technical Context

**Language/Version**: Rust 2021 workspace；TypeScript 5 + React 18  
**Primary Dependencies**: `tokio`、`axum`、`serde`、`serde_json`、`uuid`、`chrono`、`tracing`、`dashmap`；前端使用 `vite` 5、`vitest`、`eslint`、`react-markdown`  
**Storage**: append-only JSONL session event logs + runtime 内存态 agent control / 交付缓冲 + HTTP/SSE 历史投影  
**Testing**: `cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`、`cd frontend && npm run typecheck && npm run lint && npm run format:check && npm run test`  
**Target Platform**: Tauri 桌面应用 + 本地 HTTP/SSE 服务 + React SPA  
**Project Type**: Rust workspace backend + React frontend + Tauri shell  
**Performance Goals**: 新子智能体 100% 使用独立子会话；resume 100% 沿用原子会话身份；在支持缓存指标的 provider 上，重复启动相似子智能体时 `cache_creation_input_tokens` 相较首次下降至少 70%；父历史不混入子内部事件；遇到旧共享写入历史时快速失败而不是进入双轨逻辑  
**Constraints**: 不需要向后兼容；旧共享写入历史必须显式拒绝而不是兼容读取；不得把 `ReactivationPrompt` 或交付详情写进父 durable 消息流；父背景必须经 prompt 层注入；运行时 overlay 可在重启后消失，但 durable 真相不可丢；不得新增 panic 路径、fire-and-forget 任务或持锁 await；`runtime` 门面单文件不得突破仓库约束  
**Scale/Scope**: 影响 `crates/core`、`crates/storage`、`crates/runtime-session`、`crates/runtime-execution`、`crates/runtime-agent-control`、`crates/runtime-agent-loop`、`crates/runtime-agent-tool`、`crates/runtime-prompt`、`crates/runtime-llm`、`crates/runtime`、`crates/protocol`、`crates/server` 以及 `frontend/src`

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

本 feature 同时命中 durable 历史行为变化、公共 runtime surface 语义收紧、跨边界职责调整三类高风险项，因此必须生成 `findings.md`、`design-*.md` 和 `migration.md` 的三层文档，而不是只写轻量计划。

### Pre-Phase Gate

- **Durable Truth First**: PASS。计划把 child session JSONL、父侧边界事实和 lineage 查询来源明确区分；运行时唤醒、交付缓冲和活动观测仅作为 overlay；旧共享写入历史不再作为可回放真相。
- **One Boundary, One Owner**: PASS。`runtime-session/storage` 持有 durable 真相与 legacy 拒绝，`runtime-execution` 负责 spawn/resume/delivery 编排，`runtime-agent-loop` 与 `runtime-agent-control` 持有运行时唤醒与缓冲，`runtime-prompt` 持有 inherited blocks 与 fingerprint/cache，`server/frontend` 只消费投影。
- **Protocol Purity, Projection Fidelity**: PASS。计划要求把 durable 事件、领域边界事实、协议 DTO 与 `/history`、`/events` 投影语义分别建模，不把 runtime 内部桥接结构泄漏到 `protocol`，也不再为 legacy 共享历史提供协议级兼容投影。
- **Ownership Over Storage Mode**: PASS。子智能体所有权由父会话、子会话、执行实例和边界事实定义，不由磁盘目录嵌套决定；共享写入模式不再拥有持续语义地位。
- **Explicit Migrations, Verifiable Refactors**: PASS。计划包含 caller inventory、阶段化 cutover、旧路径移除顺序与完整验证命令；兼容不是默认义务。
- **Runtime Robustness**: PASS。计划显式处理当前空状态 resume、durable `ReactivationPrompt`、进程内缓冲丢失和投递去重等风险点，并要求替换为可恢复、可观测实现。
- **Observability & Error Visibility**: PASS。计划要求 child session 创建、resume、fingerprint 命中/失效、父唤醒、交付缓冲、legacy 拒绝与 lineage 不一致都有结构化日志和明确错误投影。

### Post-Phase Re-Check

- **Durable Truth First**: PASS。`research.md`、`data-model.md` 和 `design-runtime-boundaries.md` 已把 durable 真相、运行时桥接和读模型区分清楚，并明确 legacy 共享历史不再属于支持范围。
- **One Boundary, One Owner**: PASS。`design-runtime-boundaries.md` 为各 crate 指定单一 owner，`migration.md` 明确了旧职责的删除顺序。
- **Protocol Purity, Projection Fidelity**: PASS。`contracts/session-history-and-child-notifications.md` 与 `contracts/agent-collaboration-tools.md` 收紧了跨边界契约，`contracts/prompt-inheritance-and-cache-observability.md` 收紧了 prompt 与 telemetry 可观察面。
- **Ownership Over Storage Mode**: PASS。`data-model.md` 明确 lineage 是基于 durable 事实重建的查询模型，不引入新的全局真相表，也不再为共享写入模式维护额外 owner 语义。
- **Explicit Migrations, Verifiable Refactors**: PASS。`migration.md` 给出逐阶段切换顺序、移除 legacy 路径的门槛和验收命令。
- **Runtime Robustness**: PASS。设计明确禁止用空状态 resume、禁止 durable `ReactivationPrompt`、要求缓冲幂等和恢复可追溯。
- **Observability & Error Visibility**: PASS。设计要求日志、边界错误事实和 `PromptMetrics` 指标共同覆盖本 feature 的关键路径。

## Project Structure

### Documentation (this feature)

```text
specs/004-subagent-cache-context/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── findings.md
├── design-runtime-boundaries.md
├── design-prompt-cache-and-context.md
├── migration.md
├── contracts/
│   ├── agent-collaboration-tools.md
│   ├── prompt-inheritance-and-cache-observability.md
│   └── session-history-and-child-notifications.md
└── tasks.md
```

使用三层文档的原因：

- 会改变 child session / parent history 的 durable 行为
- 会收紧 `spawnAgent` / resume / history-projection 等公共 runtime surface 语义
- 会调整 `runtime-execution`、`runtime-agent-loop`、`runtime-agent-control` 与 `runtime-prompt` 的职责边界
- 会显式删除 legacy 共享写入读取/回放/恢复路径

### Source Code (repository root)

```text
crates/
├── core/                         # 领域契约、事件接口、环境常量、跨边界 DTO
├── storage/                      # JSONL 持久化与会话路径解析
├── runtime-session/              # 会话 durable truth、replay、会话加载/保存、legacy 拒绝
├── runtime-execution/            # child spawn/resume/delivery 编排、context snapshot
├── runtime-agent-control/        # live agent registry、父唤醒缓冲、投递去重
├── runtime-agent-loop/           # 活跃 agent turn 消费、唤醒衔接、子完成处理
├── runtime-agent-tool/           # `spawnAgent` 等协作工具适配层
├── runtime-prompt/               # fingerprint、LayerCache、PromptDeclaration 组装
├── runtime-llm/                  # provider telemetry，包括 cache metrics
├── runtime/                      # façade 组装与对外服务入口
├── protocol/                     # HTTP/SSE DTO
├── server/                       # `/history`、`/events`、session/status/projection API
└── runtime-registry/             # capability/tool 路由与装配

frontend/
└── src/
    ├── components/               # 父子会话 UI
    ├── hooks/                    # session / SSE orchestration
    ├── lib/                      # API client 与投影适配
    └── store/                    # 前端状态管理

src-tauri/                        # 桌面壳
```

**Structure Decision**: 保持现有 Rust workspace + React + Tauri 结构，不引入新服务、新数据库或独立消息队列。此次实现只在现有 crate 边界内重排 ownership 与投影语义，并显式删除 legacy 共享写入路径。

## Complexity Tracking

本计划没有申请宪法例外。复杂度限制如下：

- 不引入第二套 durable 存储系统
- 不新增“把机制当消息写进 durable log”的兼容层
- 不为了缓存复用在规格里重造一套独立 fingerprint 体系
- 不把 lineage 提升为新的全局写入真相表
- 不保留旧共享写入模式的读取、回放或恢复双轨逻辑

## Phase 0 Research

Phase 0 已收敛并写入 [research.md](./research.md)，核心决策如下：

1. 新子智能体默认进入独立子会话，并显式拒绝旧共享写入历史。
2. resume 必须基于 child session durable replay 或等价 projector 恢复可见状态。
3. 父唤醒走运行时信号，durable log 只保留子交付等边界事实。
4. 父背景通过 `PromptDeclaration -> system blocks` 传递，不进入消息流。
5. 缓存失效边界委托给 `runtime-prompt` fingerprint 体系与共享 LayerCache。
6. recent tail 过滤优先采用确定性规则，不额外引入推理回合。
7. provider 侧缓存指标不一致时，以可观察指标能力作为验收前提。

## Phase 1 Design Outputs

- [findings.md](./findings.md): 记录当前仓库中与本 feature 直接相关的真实代码现状和风险接缝。
- [data-model.md](./data-model.md): 定义 durable 实体、运行时桥接结构和前端/服务端投影模型。
- [design-runtime-boundaries.md](./design-runtime-boundaries.md): 划分 `runtime-session`、`runtime-execution`、`runtime-agent-loop`、`runtime-agent-control`、`server` 的职责。
- [design-prompt-cache-and-context.md](./design-prompt-cache-and-context.md): 定义 inherited context、shared LayerCache、recent tail 裁剪与 provider 观测设计。
- [migration.md](./migration.md): 列出 cutover 顺序、caller inventory、旧路径移除步骤和验证门槛。
- [contracts/agent-collaboration-tools.md](./contracts/agent-collaboration-tools.md): 收紧子协作 surface 的可观察语义。
- [contracts/session-history-and-child-notifications.md](./contracts/session-history-and-child-notifications.md): 收紧 `/history`、`/events` 和父子边界事实投影。
- [contracts/prompt-inheritance-and-cache-observability.md](./contracts/prompt-inheritance-and-cache-observability.md): 收紧 prompt 继承与缓存可观察契约。
- [quickstart.md](./quickstart.md): 提供验证命令和手工验收场景。

## Phase 2 Implementation Strategy

### Workstream 1: 独立子会话 durable 真相与 legacy 路径删除

- 去除新子智能体的 `IndependentSession` experimental 阻塞。
- 把新写入统一切到独立 child session durable 历史。
- 删除旧共享写入模式的读取、回放与恢复路径。
- 在遇到旧共享写入历史时快速返回 `unsupported` 或 `upgrade required`。

### Workstream 2: Replay-Based Resume 与 Lineage 恢复失败

- 用 child session durable replay 或 projector 恢复下一轮执行的可见状态。
- 在 resume 成功时保留原子会话身份、生成新执行实例。
- 在 lineage 不一致或历史损坏时输出双通道错误事实并失败返回。

### Workstream 3: Prompt 继承与共享缓存边界

- 将父 compact summary / recent tail 提升为独立 inherited prompt blocks。
- 在 `runtime-prompt` 中复用现有 fingerprint 与 LayerCache。
- 为 recent tail 增加确定性筛选、预算裁剪和工具输出摘要。

### Workstream 4: 父唤醒与交付桥接

- 移除 durable `ReactivationPrompt` 驱动路径。
- 使用运行时唤醒信号和一次性交付输入完成父 turn 衔接。
- 对多子交付引入独立缓冲、幂等消费和重启后可追溯约束。

### Workstream 5: 投影、协议、前端与观测

- 调整父历史、子会话入口和 status/projection DTO。
- 确保 `/history` 与 `/events` 对新边界事实语义一致。
- 对不受支持的 legacy 共享历史返回稳定错误。
- 更新前端父摘要/子会话入口展示。
- 为 cache reuse、resume、lineage mismatch、delivery wake 和 legacy rejection 增补日志和测试。

## Validation Gate

实现完成后至少需要通过以下门槛：

1. `cargo fmt --all --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --exclude astrcode`
4. `cd frontend && npm run typecheck && npm run lint && npm run format:check && npm run test`
5. 快速手工验证独立子会话、resume 沿用原会话、缓存指标下降、无 durable `ReactivationPrompt`、多交付缓冲，以及 legacy 共享历史被显式拒绝
