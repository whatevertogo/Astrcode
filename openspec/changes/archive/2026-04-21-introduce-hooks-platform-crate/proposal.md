## Why

Astrcode 现在只有一套很窄的 `core::hook` 契约，事件面只覆盖工具调用与 compact，无法承载 session、turn、permission、workflow、subagent 等更完整的 agent 生命周期扩展；与此同时，内置系统自己的 prompt / workflow 特殊逻辑仍散落在 `application` 层硬编码分支里，外部扩展和内置行为没有共享同一平台。现在需要把 hooks 升格成独立 crate 的生命周期扩展平台，同时把 `core` 收缩为极薄的共享语义面，而不是继续让完整 hooks 运行时机制停留在 `core`。

## What Changes

- 新增独立 `astrcode-hooks` crate，承载 hooks 平台的事件模型、typed payload、effect、matcher、registry、runner、report 与 schema。
- 将当前 `crates/core/src/hook.rs` 收缩为极小的共享语义面或兼容壳层；完整的 hooks 平台运行时不再写入 `core`。
- 引入统一的 builtin / external hook 注册模型：内置系统自己的 plan / workflow / permission / compact 等行为也通过同一 hooks 平台实现，而不是继续走硬编码特例。
- 将 turn 级 prompt/context 注入收敛为标准 hook effect，通过现有 `PromptDeclaration` / governance surface 链路进入 prompt 组装，不新增平行 prompt 渲染系统。
- 扩展 plugin hook 接入与 reload 语义，使 plugin hooks 与 builtin hooks 一起进入统一 registry，并具备一致的 candidate snapshot / commit / rollback 行为。
- **BREAKING**：现有 plugin hook 适配路径不再直接围绕 `core::HookHandler` 扩展，而是迁移到 hooks 平台的事件与 schema；现有窄版 hook API 只保留最小兼容层，不再承载平台演进。
- 吸收并替代当前更窄的 `extract-governance-prompt-hooks` 方向：prompt hooks 不再作为单独平行系统推进，而是成为 hooks 平台中的标准 turn-level effect。

## Capabilities

### New Capabilities
- `lifecycle-hooks-platform`: 定义独立 hooks crate、生命周期事件模型、effect 约束、builtin/external handler 类型、执行顺序、失败语义与 hook observability

### Modified Capabilities
- `plugin-integration`: plugin hook 的声明、注册、调用与热重载需要从 `core::HookHandler` 适配升级为 hooks 平台协议
- `plugin-capability-surface`: plugin hooks 需要与 builtin hooks、skills、capabilities 一起参与统一候选快照与重载一致性
- `governance-surface-assembly`: 所有 turn 入口需要在治理装配阶段执行 turn-level hooks，并把合法 hook effect 合并进治理包络
- `mode-prompt-program`: mode / builtin prompt 行为需要通过 hooks 平台的 turn-level prompt effects 进入既有 `PromptDeclaration` 注入路径
- `workflow-phase-orchestration`: workflow phase 相关 overlay 与 lifecycle 事件需要通过 hooks 平台暴露，但 hooks 不得接管 signal 解释或 phase 迁移真相

## Impact

- 受影响代码：
  - 新增 `crates/hooks`
  - `crates/core/src/hook.rs`
  - `crates/application/src/governance_surface/*`
  - `crates/application/src/session_use_cases.rs`
  - `crates/application/src/session_plan.rs`
  - `crates/application/src/workflow/*`
  - `crates/server/src/bootstrap/governance.rs`
  - `crates/protocol/src/plugin/*`
  - plugin / supervisor / reload 相关模块
- 用户可见影响：
  - 默认行为应保持等价，但系统将具备更完整的 hook 生命周期扩展能力，并允许在更多 agent 生命周期点上注入上下文、阻止操作或附加系统消息
- 开发者可见影响：
  - 后续内置特性与插件扩展不再各自实现私有 hook 逻辑，而是统一注册到 hooks 平台
  - prompt/context 注入、permission request、tool pre/post processing、subagent 生命周期回调将共享同一套 effect 与 observability 模型
- 系统边界影响：
  - 需要同步更新 `PROJECT_ARCHITECTURE.md`，明确 `astrcode-hooks` 作为平台 crate 的职责，以及它与 `core`、`application`、`server`、plugin 协议层的边界
  - `core` 只保留最小共享语义，不拥有 hooks 平台的 registry、runner、reload、report、schema 与执行语义
  - `extract-governance-prompt-hooks` 应视为被本 change 吸收，避免并行演进两套 hook 系统
