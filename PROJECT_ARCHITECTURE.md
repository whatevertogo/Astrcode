# 项目架构总览

本文档是仓库级架构的权威说明。`README.md`、`docs/architecture/*` 与各专题文档可以展开局部细节，但不得与本文档的分层边界和依赖方向冲突。

## 核心分层

系统分为四层需要明确区分的语义：

1. `mode envelope`
   `mode` 只负责治理信封：能力面、策略、子代理规则、prompt program、执行限制。`mode` 不表达完整业务流程，也不拥有跨 turn 的正式工作流状态。
2. `workflow phase`
   `workflow` 负责正式工作流编排。`phase` 是 workflow 的执行单元，声明当前业务角色、绑定的 `mode_id`、允许的 signal/transition，以及跨 phase 的 bridge context。`phase` 复用 mode，但不重建 mode catalog。
3. `application orchestration`
   `application` 是正式工作流和用例编排入口。它解释 active workflow、phase overlay、用户 signal 与迁移时机，然后通过稳定 runtime 合同驱动 session 执行。
4. `session-runtime truth`
   `session-runtime` 是单 session 的执行引擎和事实边界。它只持有 turn lifecycle、event projection、query/read model 与恢复语义，不承载 workflow 业务编排。

## 职责边界

### `core`

- 定义领域协议和跨 crate 共享的纯数据模型。
- `CapabilitySpec` 是运行时内部能力语义真相。
- `WorkflowDef`、`WorkflowPhaseDef`、`WorkflowTransitionDef`、`WorkflowBridgeState` 等 workflow 协议也属于这一层。
- `core` 不依赖 `application`、`session-runtime` 或任何 adapter。

### `application`

- 是唯一的业务编排入口。
- 负责解释 active workflow、phase signal、phase overlay、artifact bridge 与 mode 切换顺序。
- 只通过 `session-runtime` 暴露的稳定 command/query 合同消费会话事实。
- 不直接操作 execution lease、event append helper、display `Phase` lock 或 runtime 内部 shadow state。

### `session-runtime`

- 是单 session 执行与恢复的 authoritative truth。
- 内部只保留两类状态：
  - runtime control state：active turn、cancel、lease、deferred compact 等进程内控制信息
  - projection/read-model state：由 durable event 增量投影得到的 phase、mode、turn terminal、active tasks、child session、input queue 等事实
- display `Phase` 只由 durable event 投影驱动，不允许被运行时代码直接写入。
- workflow state 不属于 `session-runtime` 内部事实。

### `server`

- 是唯一组合根。
- 组装 `application`、`session-runtime`、`kernel` 与各 adapter。
- 不承载业务真相，只负责装配和协议映射。

## `mode envelope` 与 `workflow phase` 的关系

- `mode` 负责治理约束，回答“这一轮允许做什么、如何做”。
- `workflow phase` 负责业务语义，回答“当前处于正式流程的哪一段、下一步如何迁移”。
- 同一个 `mode_id` 可以被多个 phase 复用。
- workflow 迁移必须通过显式 `transition` 与 `bridge` 建模，不能继续散落在提交入口的 plan-specific if/else 里。

## `application` 与 `session-runtime` 的边界

- `application -> session-runtime` 是单向依赖。
- `session-runtime` 不反向依赖 `application`，也不解释 approval、replan、plan bridge 等 workflow 业务语义。
- `application` 通过稳定 facade 推进一次 turn、切 mode、读取 authoritative snapshot。
- `session-runtime` 内部的 `TurnCoordinator`、projection registry、checkpoint 与 event translator 都属于 runtime 子域实现细节，不应被 `application` 直接持有。

## 依赖方向

仓库级依赖方向保持如下不变式：

- `server` 是组合根，可以依赖 `application`、`session-runtime`、`kernel` 和 adapter。
- `application` 只依赖 `core`、`kernel`、`session-runtime`。
- `session-runtime` 只依赖 `core`、`kernel`。
- `protocol` 只依赖 `core`。
- `adapter-*` 只实现端口，不拥有业务真相。
- `src-tauri` 是桌面薄壳，不承载业务逻辑。

## 事件与恢复语义

- event log 仍是执行时间线的 durable truth。
- display phase、mode、turn terminal、active tasks、child session、input queue 等派生事实必须能由事件投影恢复。
- workflow instance state 是独立于 runtime checkpoint 的显式持久化状态；workflow 恢复失败时允许降级到 mode-only 路径，但不应阻塞 session-runtime 恢复。

## 文档关系

- 本文档：仓库级分层边界与依赖方向的权威约束。
- `README.md`：项目介绍和对外说明。
- `docs/architecture/crates-dependency-graph.md`：crate 依赖图和结构快照。
- `docs/特点/*`：专题设计与局部机制说明。
