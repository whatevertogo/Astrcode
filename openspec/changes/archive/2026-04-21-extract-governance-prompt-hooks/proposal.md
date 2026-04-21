## Why

当前 `plan` / workflow 相关 prompt 注入逻辑分散在 `session_plan.rs` 与 `session_use_cases.rs` 的条件分支里，`facts`、`reentry`、`exit`、`execute bridge` 等声明既没有统一抽象，也无法被其他 mode 或 workflow 复用。继续在这个结构上推进 mode contract 重构，会把新的 mode 语义继续绑死在 plan 专属 helper 上，导致后续 `enterMode` / `exitMode` / mode artifact 通用化缺少稳定落点。

现在先做 hooks refactor，是为了把“谁可以基于 session / artifact / workflow 状态注入 prompt declarations”沉淀成独立能力，并把 plan 的特殊逻辑从硬编码 helper 收回到可组合 hook 边界里。这样后续 mode change 可以只关注 mode contract 与工具面，不必同时搬运 prompt 侧的遗留结构。

## What Changes

- 新增 governance 级 prompt hook 能力，定义 turn 提交前如何基于 session、artifact、workflow 与 mode 上下文解析额外 `PromptDeclaration`。
- 将 builtin `plan` mode 当前的 `facts` / `reentry` / `template` / `exit` / `execute bridge` prompt 逻辑迁移到 hook 解析路径，不再由 `session_use_cases` 直接拼接专用 helper。
- 让 workflow phase 的 bridge prompt overlay 通过 workflow-scoped hook/provider 产出，而不是在提交路径里按 phase 写死条件分支。
- 保持现有 `PromptDeclaration -> PromptPlan` 注入链路不变；本 change 不引入新的 prompt 渲染 IR，也不在本 change 中做 mode 工具通用化。
- 明确与现有 `core::hook::HookHandler` 的分层：新的 governance prompt hooks 只负责 turn-scoped prompt declaration 解析，不介入工具调用或 compact 生命周期。

## Capabilities

### New Capabilities
- `governance-prompt-hooks`: 定义 governance/application 层如何注册、解析和组合 turn-scoped prompt hooks，以生成额外的 `PromptDeclaration`

### Modified Capabilities
- `mode-prompt-program`: mode prompt program 需要支持通过 governance prompt hooks 扩展 builtin mode 的动态 prompt 输入，同时继续走既有 `PromptDeclaration` 注入路径
- `workflow-phase-orchestration`: workflow phase 的 bridge prompt 与 phase-specific overlay 需要从 workflow prompt hook/provider 产出，而不是散落在 plan-specific 条件分支中

## Impact

- 受影响代码：
  - `crates/application/src/session_plan.rs`
  - `crates/application/src/session_use_cases.rs`
  - `crates/application/src/workflow/*`
  - `crates/application/src/mode/*`
  - `crates/application/src/governance_surface/*`
  - `crates/core/src/hook.rs`（仅需明确边界，不预期复用现有生命周期 hook trait）
- 用户可见影响：
  - 默认 `plan` / `plan_execute` 行为应保持等价，但 prompt 来源会从专用 helper 切换为 hook 解析路径
- 开发者可见影响：
  - 后续新增 mode/workflow prompt 行为时，不再修改 `session_use_cases` 主提交流程，而是在 governance prompt hook 边界内扩展
- 架构影响：
  - 需要补充 `PROJECT_ARCHITECTURE.md` 或配套架构文档，明确“lifecycle hooks” 与 “governance prompt hooks” 是两套不同粒度的扩展机制
