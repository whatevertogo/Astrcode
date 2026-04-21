## Context

Astrcode 现在已经有两套与“hook”相关但粒度完全不同的机制：

1. `crates/core/src/hook.rs` 中的 `HookHandler`
   - 面向 `PreToolUse` / `PostToolUse` / `PreCompact` / `PostCompact`
   - 解决插件在工具调用与 compact 生命周期上的扩展
   - 输入输出强类型，但触发点是单次工具调用或 compact

2. `session_plan.rs` / `session_use_cases.rs` 中的 plan/workflow prompt helper
   - 面向 turn 提交前的 prompt declaration 生成
   - 负责 `facts`、`reentry`、`template`、`exit`、`execute bridge`
   - 当前没有统一抽象，逻辑散落在提交流程的 if/else 中

这两类机制名称相近，但职责边界不同。当前真正缺的是第二类：一个 application 层、turn-scoped、只负责解析额外 `PromptDeclaration` 的扩展点。没有这层，后续 mode contract 重构会继续把 prompt 侧语义绑在 `plan` 专属 helper 上，导致 mode change 既要处理工具面，又要清理 prompt 遗留硬编码。

当前状态的主要问题：

- `session_use_cases.rs` 同时负责：
  - session / workflow 状态恢复
  - signal 解释与 phase 迁移
  - mode reconcile
  - plan/workflow prompt declaration 拼装
- `session_plan.rs` 既承载 plan artifact 真相，又承载 mode/workflow prompt helper
- `mode-prompt-program` 规范只定义了 mode prompt program 与 `PromptDeclaration` 注入路径，但没有正式描述“运行时动态 prompt 输入应如何在 application 层解析”
- `workflow-phase-orchestration` 已明确 workflow 与 `HookHandler` 分层，却还没有自己的 prompt overlay 解析边界

这次 change 的目标是先把第二类能力抽出来：形成一个可以承载 builtin plan/workflow prompt 行为的 governance prompt hook 系统。它是后续 mode change 的前置基础设施，但本次不做 mode contract、通用工具或 plugin mode 注册扩展。

与 `PROJECT_ARCHITECTURE.md` 的关系：

- 本次不改变 `server -> application -> session-runtime` 依赖方向。
- 新增的 hook 边界仍位于 `application`，不下沉到 `core`、`kernel` 或 `session-runtime`。
- 需要同步补充文档，明确 `HookHandler` 与 `Governance Prompt Hooks` 是两套不同粒度的扩展机制。

## Goals / Non-Goals

**Goals:**

- 建立 application 层的 governance prompt hook 抽象，统一 turn 提交前额外 `PromptDeclaration` 的解析入口。
- 让 builtin `plan` mode 的 `facts` / `reentry` / `template` / `exit` prompt 迁移到 hook 解析路径。
- 让 workflow `plan -> execute` bridge prompt 通过 workflow-scoped hook/provider 产出，而不是散落在提交分支中。
- 保持现有 `PromptDeclaration -> PromptPlan` 组装链路不变，不引入新的 prompt 渲染 IR。
- 为后续 mode change 预留稳定接口，使 mode contract 可以直接复用 prompt hooks，而不需要再次改造提交流程。

**Non-Goals:**

- 不把现有 `core::HookHandler` 泛化成 governance prompt hooks。
- 不在本次引入 plugin 可注册的 prompt hook 协议。
- 不在本次做 `enterMode` / `exitMode` / `upsertModeArtifact` 通用化。
- 不修改 `PromptDeclarationContributor`、`PromptPlan` 或 adapter-prompt 的渲染协议。
- 不让 hook 自己承担持久化、事件写入、workflow 迁移或 mode 切换职责。

## Decisions

### 决策 1：新增 application 层 `governance prompt hooks`，而不是复用 `core::HookHandler`

选择：

- 在 `crates/application` 内新增独立模块，例如 `prompt_hooks/`
- 定义只面向 turn-scoped prompt 解析的 trait / resolver
- 不复用 `core::HookHandler` trait，不把 prompt overlay 伪装成 tool/compact lifecycle hook

原因：

- `HookHandler` 的触发点是工具调用与 compact，属于插件扩展面；governance prompt hooks 的触发点是 turn 提交边界，属于 application orchestration。
- 如果强行复用 `HookHandler`，会把 workflow / mode prompt 语义拉进 `core`，破坏当前清晰的分层。
- prompt overlay 的输入依赖 session、artifact、workflow state，这些真相本来就在 `application` 层，不应反向上提到 `core`。

备选方案：

- 直接扩展 `HookEvent`，增加 `PrePromptSubmit`
  - 未采纳原因：会把 application 业务编排钩子和插件生命周期钩子混成一套系统，边界错误。

### 决策 2：hook 输入使用 typed submission context，由 orchestration 预先装配，不让 hook 自己做隐藏 I/O

选择：

- 定义 `GovernancePromptHookInput` 之类的强类型输入
- 输入由 `session_use_cases` / workflow orchestration 预先装配
- hook 只根据输入决定是否产出 `PromptDeclaration`

建议形状：

```rust
pub enum GovernancePromptHookInput {
    ModeActive(ModeActivePromptContext),
    ModeExit(ModeExitPromptContext),
    WorkflowPhaseOverlay(WorkflowPhasePromptContext),
}
```

其中上下文至少包含：

- `session_id`
- `working_dir`
- 当前 `mode_id`
- 用户提交文本（如需要）
- 已加载的 plan / workflow / bridge 摘要
- 其他 hook 决策所需的纯数据事实

原因：

- turn 提交路径对性能和一致性敏感，隐藏 I/O 会让 hook 的错误边界不可控。
- 让 orchestration 统一读取状态，再把纯数据喂给 hooks，可以保证 resolver 是纯解析器，而不是半个 service layer。
- 这也让 hooks 更容易测试：无需搭建文件系统，只需构造 typed context。

备选方案：

- 让 hook 持有 `session_plan` / `workflow_state_service` 等依赖，自行读取状态
  - 未采纳原因：会把 resolver 变成服务定位器，增加状态读取重复与错误传播复杂度。

### 决策 3：初始只定义三类 hook 触发点，覆盖当前 plan/workflow 真实需求

选择：

- `ModeActive`
  - 解决 plan mode 的 `facts` / `reentry` / `template`
- `ModeExit`
  - 解决 plan 批准后的 exit prompt
- `WorkflowPhaseOverlay`
  - 解决 executing phase 的 bridge prompt，以及后续 phase-specific overlay

原因：

- 这是当前代码里真实存在的三类 prompt 产出时机。
- 先把现有专用 helper 抽平，比一次性设计成任意事件总线更稳。
- 后续 mode change 可以在不破坏 resolver 的前提下，把 `ModeActive` / `ModeExit` 的 builtin hook 绑定到 mode contract 上。

备选方案：

- 一开始引入更抽象的 `BeforeSubmission` / `AfterTransition` / `AfterReconcile` 事件总线
  - 未采纳原因：会过度设计，且当前没有足够多的 hook 消费者证明这些阶段都需要独立存在。

### 决策 4：builtin `plan` / workflow prompt 迁移为 hook provider，`session_plan` 只保留 artifact 与 workflow 事实

选择：

- 新增 builtin hook provider，例如：
  - `PlanModePromptHook`
  - `PlanExitPromptHook`
  - `PlanExecuteBridgePromptHook`
- `session_plan.rs` 继续保留：
  - plan artifact 路径规则
  - plan 状态模型
  - approval / signal 解析
  - workflow bridge payload 构造
- 从 `session_plan.rs` 移出：
  - `build_plan_prompt_declarations`
  - `build_plan_exit_declaration`
  - `build_execute_bridge_declaration`

原因：

- `session_plan` 应该维护 plan artifact truth，而不是长期拥有 prompt 组装职责。
- 将 prompt helper 迁移为 builtin hook provider 后，`session_use_cases` 只需准备上下文并调用 resolver，不再知道 plan prompt 的细节。
- 这一步可以直接减少 mode/workflow prompt 逻辑在提交流程中的分支复杂度。

备选方案：

- 保持 `session_plan.rs` 为 helper 集合，只在 `session_use_cases` 外再包一层 resolver
  - 未采纳原因：那只是移动调用点，仍然没有真正拆开 truth 与 prompt 解析职责。

### 决策 5：hook 输出继续通过现有 `PromptDeclaration` 链路进入治理装配

选择：

- governance prompt hooks 输出 `Vec<PromptDeclaration>`
- 仍通过 `SessionGovernanceInput.extra_prompt_declarations` 进入 `GovernanceSurfaceAssembler`
- 继续由 adapter-prompt 渲染为 `PromptPlan`

原因：

- 现有 `PromptDeclarationContributor` 与 `PromptPlan` 已经是稳定的组装出口，不需要平行 IR。
- 这样能保证 hooks refactor 行为等价，避免同时撬动 prompt renderer。
- 后续 mode contract 只需决定哪些 hook 生效，不需要再重新设计渲染协议。

备选方案：

- 先引入新的 prompt hook result IR，再二次转换为 `PromptDeclaration`
  - 未采纳原因：对当前问题没有净收益，只会增加概念重叠。

### 决策 6：hook resolver 采用确定性顺序与显式 diagnostics，但不吞并 workflow 恢复策略

选择：

- resolver 使用稳定注册顺序执行 hook
- hook 返回：
  - `declarations`
  - 可选 `diagnostics`
- workflow/state 恢复失败仍由既有 orchestration 决定是否降级；resolver 不负责 fallback 决策

原因：

- prompt overlay 的顺序会影响模型行为，必须确定性。
- diagnostics 有助于后续观测“为什么某个 hook 没产出 prompt”，但不应替代恢复策略。
- 当前 corrupted/invalid workflow state 的降级逻辑已经在 `session_use_cases`，不应该搬进 hooks。

备选方案：

- 让 hook 自己决定是否回退到 mode-only 路径
  - 未采纳原因：会让恢复语义分散到多个 hook 中，破坏单一事实源。

### 决策 7：本次只做 builtin hooks，不暴露 plugin 注册协议

选择：

- resolver 和 trait 设计为未来可扩展
- 但本次只注册 application 内建 hook providers

原因：

- 当前最紧迫的问题是清理 builtin plan/workflow 的硬编码，不是开放新的插件面。
- 若现在同时扩到插件协议，会把协议、reload、一致性、沙箱安全一起引入，扩大 change 范围。
- 后续 mode change 若需要 plugin mode 自定义 prompt，可在此基础上再定义 host 消费与注册协议。

备选方案：

- 一次性开放 plugin prompt hook 注册
  - 未采纳原因：时机过早，且当前还没有 mode contract 作为稳定挂载点。

## Risks / Trade-offs

- [风险] 新增一套 prompt hook 抽象后，名称上容易与现有 `HookHandler` 混淆
  - Mitigation：文档和代码中统一使用 `governance prompt hooks` 命名，并在 `PROJECT_ARCHITECTURE.md` 中单独写清与 lifecycle hooks 的差异。

- [风险] `session_use_cases` 重构过程中，plan/workflow prompt 行为可能发生细小回归
  - Mitigation：保留现有行为等价测试，并新增 hook resolver 的单元测试和端到端提交流程测试。

- [风险] 过早泛化 hook 输入，导致后续 mode change 仍需重写
  - Mitigation：输入只覆盖当前三类真实触发点，避免引入任意 payload 黑箱。

- [风险] builtin hooks 仍可能间接依赖 plan/workflow 内部类型，造成模块耦合
  - Mitigation：由 orchestration 先把状态收敛成最小 typed context，hook 不直接依赖持久化服务或文件系统。

- [风险] 如果未来 plugin 也要接入 prompt hooks，当前 internal trait 可能需要再次调整
  - Mitigation：本次先把职责边界抽清；后续对外协议可在不破坏内部触发点的情况下额外包一层注册适配器。

## Migration Plan

1. 先补 proposal/specs/design，固定 `governance prompt hooks` 的职责与边界。
2. 在 `crates/application` 新增 prompt hooks 模块，定义 typed input、trait、resolver 与 builtin providers。
3. 将 `session_plan.rs` 中的 prompt helper 迁移为 builtin hook provider，实现行为等价。
4. 修改 `session_use_cases.rs`，把 prompt declaration 生成改为“准备上下文 -> 调用 resolver -> 把结果传给 governance surface”。
5. 保留现有 `PromptDeclaration` 注入路径与 `GovernanceSurfaceAssembler` 接口，不在本次改动 adapter-prompt。
6. 增加单元测试与提交流程回归测试，覆盖：
   - plan 初次进入与 re-entry prompt
   - plan approval exit prompt
   - executing phase bridge prompt
   - workflow 状态损坏时的降级行为
7. 在 hooks change 完成后，再回头修订 `unify-declarative-dsl-compiler-architecture`，把其中的 prompt hook 迁移内容改成依赖本 change。

回滚策略：

- 若 hook resolver 重构引发不稳定，可保留新的 prompt hooks 模块，但让 `session_use_cases` 暂时回退到旧 helper 调用路径。
- 若 builtin hook provider 的抽象不合适，可保留 typed context，并仅将 resolver 退化为对现有 helper 的统一包装，避免完全回到 scattered branching。

## Open Questions

- 后续 mode contract 是否直接持有 hook ID / hook 模板，还是由 builtin mode catalog 在 application 层绑定 hook provider？
- workflow 如果未来出现更多 phase，`WorkflowPhaseOverlay` 是否需要细分成 `PhaseEntry` / `PhaseSteadyState` 两类输入？
- diagnostics 是否需要进入 durable observability 事件，还是先只做日志与测试可见？
