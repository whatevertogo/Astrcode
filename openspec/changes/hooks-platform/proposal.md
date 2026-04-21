## Why

项目需要为外部扩展点（plugin、内置工具、第三方集成）提供生命周期回调能力。当前 hook 系统存在三层缺失：

**运行时层面**：`core/hook.rs` 定义了 `HookEvent`、`HookInput`、`HookOutcome`、`HookHandler` trait 等基础类型，`core/policy/engine.rs` 提供了 `PolicyEngine` trait。但这些类型没有形成可用的运行时——没有注册表、没有生命周期管理、没有统一调度路径。观察型 hook（异步通知）和决策型 hook（同步阻塞）没有区分。hook 无法影响工具调用、compact 等关键行为。

**治理层面**：builtin `plan` mode 的 prompt 注入逻辑（`facts`、`reentry`、`exit`、`execute bridge`）分散在 `session_plan.rs` 与 `session_use_cases.rs` 的条件分支里，没有统一抽象，无法被其他 mode 或 workflow 复用。继续在这个结构上推进 mode contract 重构，会把新 mode 语义绑死在 plan 专属 helper 上。系统内部的行为（plan prompt、workflow overlay、permission request）和外部扩展（plugin hook）走的是两条完全不同的路径，无法共享同一个平台。

**架构层面**：完整 hooks 运行时机制不应该停留在 `core` 中。`core` 应该只保留最小共享语义面（事件类型、payload trait），hooks 平台的 registry、runner、reload、schema 与执行语义应升格为独立 crate。

前序 change 确立了必要前提：
- `linearize-session-runtime-application-boundaries`（Change 1）确立了"外部扩展点收纯数据、吐纯数据"的原则。
- `session-runtime-state-turn-boundary`（Change 2）把 turn 运行时状态完整归入 turn 子域。

这两个前提到位后，hook 系统可以安全地插入 turn 执行路径，而不需要暴露运行时内脏。

本 change 吸收并替代两个更窄的前序方向：
- `extract-governance-prompt-hooks`：plan prompt 不再作为单独平行系统推进，而是成为 hooks 平台中的标准 turn-level effect。
- `introduce-hooks-platform-crate`：独立 crate 的方向被本 change 直接采纳。

## What Changes

### 1. 独立 `astrcode-hooks` crate

- 新增 `crates/hooks` crate，承载 hooks 平台的事件模型、typed payload、effect、matcher、registry、runner、report 与 schema。
- 将 `crates/core/src/hook.rs` 收缩为极小的共享语义面或兼容壳层；完整的 hooks 平台运行时不再写入 `core`。

### 2. 统一 hook 生命周期模型

- 引入统一的 builtin / external hook 注册模型：内置系统自己的 plan / workflow / permission / compact 等行为也通过同一 hooks 平台实现，而不是继续走硬编码特例。
- 明确区分两种 hook 类型：
  - **决策型 hook**（同步阻塞）：`beforeToolCall`、`beforeModelRequest` 等。接收纯数据 context，返回纯数据 verdict（允许/拒绝/修改）。在 turn 执行路径中同步调用，结果影响后续行为。
  - **观察型 hook**（异步通知）：`afterToolCall`、`afterCompact`、`afterTurnComplete` 等。接收纯数据 context，无返回值。在 turn 执行路径后异步触发，不影响执行结果。
- 定义 hook 执行顺序、失败语义与 observability。

### 3. Turn 执行路径中的 hook 调度点

- 在 turn 执行的关键节点（tool 调用前/后、compact 前/后、turn 开始/结束）插入 hook 调度点。
- hook 的输入输出严格遵循纯数据原则——context 和 verdict 都是可序列化的 DTO，不包含 CancelToken、锁、原子变量等运行时原语。

### 4. 治理 prompt hooks（吸收 extract-governance-prompt-hooks）

- 定义 governance 级 prompt hook 能力，turn 提交前如何基于 session、artifact、workflow 与 mode 上下文解析额外 `PromptDeclaration`。
- 将 builtin `plan` mode 当前的 `facts` / `reentry` / `template` / `exit` / `execute bridge` prompt 逻辑迁移到 hook 解析路径，不再由 `session_use_cases` 直接拼接专用 helper。
- 让 workflow phase 的 bridge prompt overlay 通过 workflow-scoped hook/provider 产出，而不是在提交路径里按 phase 写死条件分支。
- turn 级 prompt/context 注入收敛为标准 hook effect，通过现有 `PromptDeclaration` / governance surface 链路进入 prompt 组装，不新增平行 prompt 渲染系统。

### 5. Plugin hook 接入

- 通过 plugin SDK 暴露 hook 注册 API，plugin 可声明自己处理哪些 hook。
- plugin hooks 与 builtin hooks 进入统一 registry，具备一致的 candidate snapshot / commit / rollback 行为。
- 扩展 plugin reload 语义：plugin hooks 参与统一 reload，与 mode catalog、capability surface、skill catalog 的切换一起满足原子替换或完整回滚。

## Non-Goals

- 本次不实现 hook 的持久化或跨 session 共享。
- 本次不实现 hook 的权限隔离（哪些 plugin 可以注册哪些 hook）。
- 本次不实现 hook 的超时、重试或熔断机制。
- 本次不直接移除 `enterPlanMode` / `exitPlanMode` / `upsertSessionPlan` 等现有工具——plan prompt 先迁入 hook 路径，工具通用化留给 `unify-declarative-dsl-compiler-architecture`。
- 本次不接管 workflow signal 解释或 phase 迁移真相。
- 本次不新增平行 prompt 渲染系统。

## Capabilities

### New Capabilities
- `lifecycle-hooks-platform`: 定义独立 hooks crate、生命周期事件模型、effect 约束、builtin/external handler 类型、执行顺序、失败语义与 hook observability。
- `governance-prompt-hooks`: 定义 governance/application 层如何注册、解析和组合 turn-scoped prompt hooks，以生成额外的 `PromptDeclaration`。

### Modified Capabilities
- `turn-execution`: turn 执行路径中增加 hook 调度点（tool 调用、compact、turn 生命周期）。
- `plugin-sdk`: SDK 新增 hook 注册 API。
- `plugin-integration`: plugin hook 的声明、注册、调用与热重载从 `core::HookHandler` 适配升级为 hooks 平台协议。
- `plugin-capability-surface`: plugin hooks 与 builtin hooks、skills、capabilities 一起参与统一候选快照与重载一致性。
- `governance-surface-assembly`: 所有 turn 入口在治理装配阶段执行 turn-level hooks，合法 hook effect 合并进治理包络。
- `mode-prompt-program`: mode / builtin prompt 行为通过 hooks 平台的 turn-level prompt effects 进入既有 `PromptDeclaration` 注入路径。
- `workflow-phase-orchestration`: workflow phase 相关 overlay 与 lifecycle 事件通过 hooks 平台暴露，但 hooks 不接管 signal 解释或 phase 迁移真相。

## Impact

- 受影响代码：
  - 新增 `crates/hooks`
  - `crates/core/src/hook.rs`（收缩为兼容壳层）
  - `crates/application/src/session_plan.rs`（plan prompt 迁移到 hook）
  - `crates/application/src/session_use_cases.rs`（移除 plan-specific 条件分支）
  - `crates/application/src/governance_surface/*`（hook 调度集成）
  - `crates/application/src/workflow/*`（workflow prompt hook）
  - `crates/session-runtime/src/turn/*`（hook 调度点插入）
  - `crates/server/src/bootstrap/governance.rs`（reload 路径）
  - `crates/protocol/src/plugin/*`（plugin hook 协议）
  - plugin / supervisor / reload 相关模块
- 新增功能，不影响现有行为：hook 注册表初始为空，所有 hook 调度点走 no-op 默认路径。plan prompt 先迁入 hook 路径并验证等价性。
- 依赖 `linearize-session-runtime-application-boundaries`（纯数据接口原则）和 `session-runtime-state-turn-boundary`（turn 运行时状态归位）的成果。
- `extract-governance-prompt-hooks` 和 `introduce-hooks-platform-crate` 被本 change 吸收，不再独立演进。
