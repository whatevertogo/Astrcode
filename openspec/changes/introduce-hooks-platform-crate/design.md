## Context

Astrcode 当前的 hooks 体系处于“够用但不可扩展”的状态：

- `crates/core/src/hook.rs` 只覆盖 `PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PreCompact`、`PostCompact`
- hook trait 与 effect 直接定义在 `core`，导致它既像共享语义，又开始滑向运行时平台
- plugin integration 仍以“把 `HookHandler` trait 适配给 plugin 调用”来表达 hook 扩展，说明 hooks 还没有成为独立平台
- 与 hooks 语义相近的 turn-level prompt / workflow overlay 逻辑，仍散落在 `session_plan.rs` 与 `session_use_cases.rs` 中，没有复用现有 hook 契约

这带来三个结构性问题：

1. **内置系统与外部扩展不共用同一平台**
   - builtin `plan` / workflow / permission 逻辑走 application 硬编码
   - plugin hooks 走 `core::HookHandler`
   - 结果是两套扩展模型并行存在

2. **`core` 被迫承载过多生命周期细节**
   - 如果继续把 session、turn、permission、subagent、workflow 等事件堆到 `core::hook`
   - `core` 会开始拥有 application orchestration 的细节，边界会脏

3. **turn-level prompt/context 注入没有正式平台**
   - 这正是之前 `extract-governance-prompt-hooks` 试图解决的问题
   - 但如果单独做一套 governance prompt hooks，很快又会和 lifecycle hooks 形成平行系统

结合 Claude Code 与 Codex 的经验，Astrcode 真正需要的不是“再补一个 prompt hooks 模块”，而是一个**独立 crate 的受约束 lifecycle extension pipeline**：

- 统一事件模型
- 统一 typed payload / effect / matcher
- 统一 builtin / external handler 注册
- 统一 observability
- 统一 turn-level prompt/context 注入效果

与此同时，Astrcode 仍需保持自己的架构边界：

- `server` 继续是组合根
- `application` 继续负责业务真相、治理装配和 lifecycle 触发
- `session-runtime` 不接管 hooks 业务编排
- policy / capability surface / governance envelope 仍是硬边界，hooks 不能绕过它们放大权限

与 `PROJECT_ARCHITECTURE.md` 的关系：

- 本次需要显式更新架构文档，把 `astrcode-hooks` 作为平台 crate 写入正式边界。
- 需要把现有 `core::hook` 从“准平台定义”收缩为“极薄共享语义 / 兼容壳层”。
- 需要说明 builtin hooks 和 external hooks 都由同一平台承载，但具体业务解释仍在 `application`。

## Goals / Non-Goals

**Goals:**

- 新增独立 `astrcode-hooks` crate，承载 hooks 平台协议与运行机制。
- 支持 builtin hooks 与 external hooks 共用同一事件、输入、effect、matcher、runner 和报告模型。
- 把 turn-level prompt/context 注入收敛为标准 hook effect，通过既有 `PromptDeclaration` / governance surface 链路生效。
- 覆盖第一阶段最关键的 lifecycle 事件：session、turn、tool、permission、compact、subagent。
- 让 plugin hook 注册、调用与 reload 切换纳入统一候选快照 / commit / rollback 模型。
- 让后续 mode/workflow 重构直接依赖 hooks 平台，而不是继续发明平行 hook 子系统。

**Non-Goals:**

- 不在本次引入新的外部 DSL 或脚本配置格式；优先复用现有配置与插件注册路径。
- 不把完整 hooks 平台重新写回 `core`。
- 不让 hooks 接管 policy truth、workflow truth 或 mode truth。
- 不让 hooks 绕过 capability surface、policy engine 或 governance envelope 放大权限。
- 不让 hooks 默认提供“任意状态突变”能力。
- 不在本次实现 `agent` handler 类型；第一阶段聚焦 `inline`、`command`、`http`。
- 不要求前端在本次立即暴露完整 hook 管理面板；前端可先只消费 observability 结果。

## Decisions

### 决策 1：`core` 只保留极薄共享语义，完整 hooks 平台落在独立 `astrcode-hooks` crate

选择：

- 新建 `crates/hooks`
- 将 hooks 平台协议、事件、payload、effect、matcher、runner、report、schema 都收敛到该 crate
- `core` 只保留极薄的共享语义类型和必要兼容壳层，不拥有 registry、runner、reload、report、schema 或执行语义

原因：

- hooks 平台需要被 `application`、`server`、plugin 协议层共同消费，它不是纯粹的 domain core 概念。
- registry、runner、reload、report、schema、顺序与失败语义都明显属于运行时平台，而不是领域真相。
- 但 `core` 又不该完全不感知共享语义，否则 hooks crate 会重新复制一层基础语义，造成漂移。
- “core 极薄、hooks 独立”比“全进 core”或“core 完全不感知”都更稳。

备选方案：

- 继续扩展 `crates/core/src/hook.rs`
  - 未采纳原因：边界会持续恶化，且无法清楚表达 hooks 平台是运行时扩展机制而不是 core 领域真相。
- 让 `core` 完全不保留任何共享语义
  - 未采纳原因：hooks crate 仍需要依赖少量稳定语义类型，完全断开会导致语义重复定义。

### 决策 2：平台只定义受限 lifecycle extension pipeline，业务真相解释仍归 `application`

选择：

- `astrcode-hooks` 定义：
  - `HookEvent`
  - `HookInput`
  - `HookEffect`
  - `HookMatcher`
  - `HookHandler`
  - `HookRegistry`
  - `HookRunner`
  - `HookExecutionReport`
- `application` 负责：
  - 在 session / turn / workflow / permission / compact / subagent 边界触发 hook
  - 将 session、workflow、mode、governance 真相收敛成 typed hook input
  - 校验和解释 hook effects

原因：

- hooks 平台应该可复用，但不能自己成为业务编排器。
- workflow phase、mode switch、permission 流程的真相仍在 `application`，hooks 只能在这些边界上观察、补充或阻止。
- 这能避免“平台协议”与“业务真相解释”互相缠绕。

备选方案：

- 让 hooks crate 直接依赖 `application` 的 context/service
  - 未采纳原因：会造成依赖反转，平台 crate 不再独立。

### 决策 3：hook point 与 effect 必须按类别约束，避免“任意事件 + 任意 effect”失控

选择：

- 将 hook point 分成三类：
  - `observe`
  - `guard`
  - `augment`
- 第一阶段不开放默认的 mutation hooks

分类表：

| 类型 | 示例 | 允许 effect | 默认失败语义 |
|---|---|---|---|
| `observe` | `PostToolUse`、`PostCompact`、`SubagentStop` | report、annotation、system note | fail-open，记录 diagnostics |
| `guard` | `PreToolUse`、`PermissionRequest` | continue、block、replace args、permission decision | 保守拒绝或按策略 fail-closed |
| `augment` | `BeforeTurnSubmit`、`SessionStart` | add prompt declarations、add context fragments、system message | fail-open，记录 diagnostics |

原因：

- 如果只有一个大 `HookEffect` enum，application 会被迫在各处写补丁式 `match` 收残局。
- 按类别约束 hook point 和 effect，后续新增事件时才有统一判断框架：它属于哪类、允许哪些 effect、失败时 open 还是 closed。
- 默认不开放 mutation hooks，可以防止平台直接篡改 session / turn / workflow 真相。

备选方案：

- 只做统一 `HookEvent` + 统一 `HookEffect`
  - 未采纳原因：过于自由，后续极易失控。

### 决策 4：内置系统与外部扩展统一注册，但 effect 权限分级

选择：

- builtin hooks 与 external hooks 共用同一 registry / runner / report 模型
- 但 effect 解释层允许按来源做权限分级：
  - builtin hooks 可使用更强的内部 typed effects
  - external hooks 默认受更严格限制

第一阶段的 effect 分级原则：

- 所有来源都可：
  - `Continue`
  - `Block`
  - `AddPromptDeclarations`
  - `AddSystemMessage`
  - `ReplaceToolArgs`（仅限 `PreToolUse`）
  - `ReplaceToolOutput`（仅限 `PostToolUse`）
  - `PermissionDecision`（仅限 `PermissionRequest`）
  - `ModifyCompactContext`（仅限 `PreCompact`）
- 只有 builtin 可直接产出需要内部 typed context 的 effect（例如 workflow bridge prompt fragments）

原因：

- 平台不能出现 builtin 和 external 两套执行模型。
- 但外部 hook 不能获得与内部业务逻辑完全同级的写权限，否则会破坏治理边界。
- “协议统一、权限分层”比“双系统”更稳，也更易观测。

备选方案：

- 完全统一权限，不区分 builtin / external
  - 未采纳原因：外部 hook 可轻易侵入内部真相，风险过高。

### 决策 5：第一阶段正式支持 12 个事件，覆盖关键 lifecycle 边界

选择：

第一阶段事件集：

- `SessionStart`
- `SessionEnd`
- `BeforeTurnSubmit`
- `PreToolUse`
- `PostToolUse`
- `PostToolUseFailure`
- `PermissionRequest`
- `PermissionDenied`
- `PreCompact`
- `PostCompact`
- `SubagentStart`
- `SubagentStop`

不在第一阶段支持但预留扩展点：

- `ModeChanged`
- `WorkflowPhaseChanged`
- `TaskCreated`
- `TaskCompleted`
- `FileChanged`
- `ConfigChanged`

原因：

- 这 12 个事件已经足够覆盖当前内置系统最迫切的切入点。
- `BeforeTurnSubmit` 可以承载 plan/workflow prompt overlay，不需要先为 prompt 单独造平台。
- 过早把 20+ 事件一次性做完，会显著扩大实现和测试面。

备选方案：

- 一次性做成 Claude 风格 20+ 事件总线
  - 未采纳原因：当前 Astrcode 还没有那么多稳定消费点，先做最有价值的骨架更稳。

### 决策 6：turn-level prompt/context 注入成为标准 hook effect，而不是单独的 prompt hooks 子系统

选择：

- `BeforeTurnSubmit` hooks 可产出 `AddPromptDeclarations`
- governance surface 在组装 turn envelope 时合并这些 declarations
- 继续沿用 `PromptDeclaration -> PromptPlan` 既有链路

原因：

- 这能直接吸收 `extract-governance-prompt-hooks` 想解决的问题。
- prompt 注入只是 hook effect 的一种，不值得单独发明第二个平台。
- 这样后续 mode/workflow/builtin prompt 行为都可以在同一 hooks 平台上表达。

备选方案：

- 保留 `governance prompt hooks` 为 application 内独立子系统
  - 未采纳原因：会再次形成 lifecycle hooks 与 prompt hooks 两套平行机制。

### 决策 7：policy / capability surface / governance envelope 是硬边界，hooks 只能收紧或补充，不能放大权限

选择：

- hook effect 解释顺序遵循：
  1. governance / policy / capability surface 先形成硬边界
  2. hooks 可以在允许范围内附加 prompt、系统消息、工具参数改写、permission 建议
  3. hooks 可以 deny/block
  4. hooks 不得扩大原始允许面

具体约束：

- `PermissionDecision::Allow` 只能在原始 verdict 为 `Ask` 时生效，不能覆写 `Deny`
- `ReplaceToolArgs` 不能把工具改写成另一类工具或跨 capability boundary 的调用
- `AddPromptDeclarations` 只能补充 prompt，不改变 governance surface 既有工具真相

原因：

- hooks 是扩展层，不是第二套治理系统。
- 如果 hooks 能放大权限，它会迅速反噬 mode/policy/governance 的确定性。

备选方案：

- 允许 hooks 完全覆写 policy 结果
  - 未采纳原因：与现有架构的治理单一事实源原则冲突。

### 决策 8：plugin hooks 通过统一 hooks registry 参与 reload，一致性模型与 capability/mode/skill 切换对齐

选择：

- reload 构建候选 hooks registry
- 与 capability surface、skill catalog、mode catalog 一起参与候选快照
- 提交时一起切换，失败时一起回滚

原因：

- hooks 现在也会影响 turn 行为，不能再作为“附属小功能”局部热替换。
- plugin hook 改变了 turn 提交、permission、tool execution 等关键边界，必须纳入统一一致性模型。
- 这也能消除当前 reload 时内置/外部行为漂移的风险。

备选方案：

- hooks registry 独立热重载，不与 capability/mode/skill 对齐
  - 未采纳原因：容易出现同一 turn 使用新 capability surface 却仍绑定旧 hooks 的不一致。

### 决策 9：hook observability 是正式产物，但 hook execution 不成为 durable truth

选择：

- 为每次 hook 执行生成结构化 `HookExecutionReport`
- 记录：
  - 事件名
  - handler 来源/类型
  - 触发时机
  - 命中/跳过
  - effect 摘要
  - 成功/失败/中止
  - 耗时
- observability 进入 runtime/application 的可观测性通道
- hook execution 本身不参与 session 恢复 replay，也不作为业务真相

原因：

- hook 是“围绕真相执行的扩展”，而不是 durable truth。
- 恢复时重新执行旧 hook 会带来副作用重复和不一致。
- 但没有报告，后续很难解释“为什么这个 turn 被 block / 为什么多了一段上下文”。

备选方案：

- 把 hook 结果写成 durable event 作为恢复事实
  - 未采纳原因：会把副作用型扩展误当成业务真相，恢复语义会非常复杂。

### 决策 10：第一阶段支持 `inline`、`command`、`http` 三类 handler，`agent` 延后

选择：

- builtin hooks 用 `inline`
- 外部本地脚本用 `command`
- 外部服务回调用 `http`
- `agent` handler 作为后续扩展，不在本 change 实现

原因：

- `agent` handler 会直接触及 subagent 生命周期、预算、治理约束和失败恢复，复杂度明显高于前三类。
- 先把平台协议、effect gating 和 observability 做稳，再接入 agent handler 更合理。

备选方案：

- 第一阶段就支持 `agent`
  - 未采纳原因：范围过大，容易把大 change 拖成无法落地的“完美架构”。

## Risks / Trade-offs

- [风险] `astrcode-hooks` 容易演化成新的垃圾桶 crate，吸走本应留在 `application` 的业务逻辑
  - Mitigation：明确平台只拥有 hook point 协议、effect 协议、执行与报告语义，不拥有业务真相、状态机、持久化真相。

- [风险] hooks 平台变大后，事件/effect 设计可能过于抽象，导致实现和测试成本飙升
  - Mitigation：第一阶段固定事件集、effect 分类和 handler 类型，先让内置需求跑通，再扩展。

- [风险] 迁移 `core::hook` 到独立 crate 时会牵动 plugin 和 application 边界
  - Mitigation：保留过渡再导出与兼容壳层，先迁协议，再迁调用点。

- [风险] builtin hooks 与 external hooks 共用平台后，外部扩展可能试图获得过强权限
  - Mitigation：effect 解释层显式做来源分级，并坚持“只能收紧不能放大”的治理约束。

- [风险] 在治理装配前后插入 `BeforeTurnSubmit` hooks，可能引入 prompt 行为微妙回归
  - Mitigation：保留行为等价测试，并为 hook-generated prompt 声明稳定顺序和 origin 标记。

- [风险] reload 一致性模型扩大后，失败回滚逻辑会更复杂
  - Mitigation：采用候选快照 + 原子提交/回滚，避免部分更新。

- [风险] 当前小 change `extract-governance-prompt-hooks` 与大 change 并行会造成冲突
  - Mitigation：在任务中显式把它并入/吸收，避免双轨实现。

## Migration Plan

1. 更新架构文档，声明 `astrcode-hooks` 的 crate 边界与职责。
2. 新建 `crates/hooks`，迁入现有窄版 hook 协议，并扩展事件、effect、matcher、registry、runner、report、schema。
3. 在 `core` 保留兼容再导出或最小壳层，避免一次性打断全部引用。
4. 在 `application` 接入第一阶段事件触发：
   - turn submit
   - tool execution
   - permission request
   - compact
   - subagent lifecycle
5. 将 `session_plan` / workflow overlay 逻辑迁移为 builtin `BeforeTurnSubmit` hooks，吸收 `extract-governance-prompt-hooks`。
6. 在 `server` 和 plugin integration 中接入统一 hooks registry 与 reload 切换。
7. 增加单元测试、集成测试、reload 失败回滚测试和 crate boundary 校验。

回滚策略：

- 若 hooks crate 迁移中断，可保留新 crate 但让旧 `core::hook` 壳层继续维持最小兼容行为。
- 若 `BeforeTurnSubmit` 接线不稳定，可暂时恢复旧 prompt helper 路径，同时保留 hooks 平台基础设施。
- 若 plugin hooks reload 不稳定，可先限制 external hooks 使用新平台，而 builtin hooks 继续先行落地。

## Open Questions

- plugin hooks 的声明协议最终是复用现有 `handlers` 描述，还是单独引入更明确的 hook descriptor？
- hook observability 是否需要在第一阶段同步暴露到前端 transcript/thread item，还是先只做后端 collector？
- `ModeChanged` / `WorkflowPhaseChanged` 是否应在第二阶段尽快补入，还是继续通过 `BeforeTurnSubmit` 上下文满足大多数场景？
