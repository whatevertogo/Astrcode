## Context

Astrcode 已经具备声明式治理与正式 workflow 的核心骨架，但当前实现把几类本应分开的真相揉在了一起：

1. `mode` 想承载更多合同语义，但 compile / bind 的边界不清。
   - `compile_mode_envelope()` 与 `GovernanceSurfaceAssembler` 的职责边界没有统一命名。
   - builtin `plan` mode 的 artifact / exit / prompt 语义仍主要体现在专用工具和 session-specific helper 中。

2. `workflow` 已经拥有 `phase -> mode` 的正式绑定点，却缺少显式的 validate / compile owner。
   - `WorkflowPhaseDef.mode_id` 已经是现有真相。
   - 但 plan approval、bridge 生成、workflow bootstrap 与 reconcile 仍散落在 `session_plan.rs`、`session_use_cases.rs` 和工具 handler 中。

3. `reload` 已经有局部原子替换，但治理输入还不是统一快照。
   - capability surface 失败时会回滚。
   - mode catalog 与 skill catalog 还没有被纳入同一次提交/回滚。

4. 工具层缺少稳定的 mode contract 读取面。
   - `ToolContext` 只有 `current_mode_id`。
   - 需要 artifact / exit 语义的工具只能硬编码规则，或者不干净地回看 application/runtime 内部实现。

这次 change 的目标不是继续扩 scope，而是把 owner 收清楚：

- `mode` 负责治理合同。
- `workflow` 负责 phase 图与 phase -> mode 绑定。
- `binder` 负责把 compile 结果与 runtime/session/profile/control 绑定。
- `tool context` 只接收 pure-data snapshot，不接触 application 内部类型。

## Goals / Non-Goals

**Goals**

- 统一 `compile`、`bind`、`orchestrate` 术语与职责边界。
- 扩展 `GovernanceModeSpec`，让 mode 能声明 artifact 合同、exit gate 与 prompt hooks。
- 明确 workflow compiled artifact 是 phase -> mode 绑定的唯一 owner。
- 为工具执行提供纯数据的 bound mode contract snapshot。
- 让 prompt 结果继续沉淀到现有 `PromptPlan`。
- 让 reload 在“无活跃 session”约束下对 mode catalog、capability surface、skill catalog 做统一候选快照提交/回滚。
- 补上 duplicate `mode_id` 冲突策略。

**Non-Goals**

- 不把 mode、workflow、prompt、capability 合并成单一 schema。
- 不把 workflow 绑定反向塞进 `GovernanceModeSpec`。
- 不在本次为 workflow 引入与当前规模不匹配的索引化结构。
- 不让 `adapter-tools` 直接依赖 `application` 或 runtime 内部类型。
- 不在本次直接设计新一代通用 mode transition DSL。

## Decisions

### 决策 1：`GovernanceModeSpec` 只扩 artifact / exit / prompt 合同，不再承载 workflow phase 绑定

选择：

- 在 `GovernanceModeSpec` 中新增：
  - `ModeArtifactDef`
  - `ModeExitGateDef`
  - `ModePromptHooks`
- 不新增 `ModeWorkflowBinding`。

原因：

- 仓库级架构已经明确 `mode` 与 `workflow phase` 是两层不同语义。
- `WorkflowPhaseDef.mode_id` 已经是 phase -> mode 绑定真相，再在 mode spec 内保存 `workflow_id/phase_id/phase_role` 只会形成双写。
- 同一个 `mode_id` 可以被多个 phase 复用；反向绑定会把这个合法关系错误收窄成一对一。

备选方案：

- 在 `GovernanceModeSpec` 中加入 `workflow_binding`
  - 未采纳原因：会复制已有 workflow 真相，并迫使 binder 做双向一致性校验。

### 决策 2：workflow compiled artifact 保持 phase -> mode 绑定 owner，mode 只提供可复用合同

选择：

- `WorkflowDef`/compiled workflow artifact 持有：
  - `phase_id`
  - `mode_id`
  - `role`
  - `artifact_kind`
  - `accepted_signals`
- workflow orchestration 通过 `phase.mode_id` 向治理编译链路索取 mode contract。

原因：

- 这符合 `PROJECT_ARCHITECTURE.md` 中“mode 负责治理约束，workflow phase 负责业务阶段”的分层。
- 可自然支持“多个 phase 复用同一个 mode”。
- recovery / reconcile 时也应该从 `current_phase_id -> phase.mode_id` 出发，而不是反向从 mode 猜 phase。

### 决策 3：compile 与 bind 保持两层产物，但为工具执行补一层 pure-data 投影

选择：

- compile 阶段产出 `CompiledModeSurface` / 等价编译产物，负责：
  - selector 求值
  - child/grant 裁剪
  - artifact / exit / prompt contract 派生
  - diagnostics
- bind 阶段产出 `ResolvedGovernanceSurface`，负责：
  - runtime config
  - resolved limits
  - profile / injected messages
  - approval pipeline
- 对工具执行额外投影一份 pure-data `BoundModeToolContractSnapshot`（命名可渐进演化），只包含工具所需的 artifact / exit 合同字段。

原因：

- `adapter-tools` 不能也不应该依赖 `GovernanceSurfaceAssembler`。
- `ToolContext` 只有 `current_mode_id` 不足以支撑 contract-aware 工具。
- 纯数据 snapshot 可以跨 `ResolvedGovernanceSurface -> AgentPromptSubmission -> ToolContext -> CapabilityContext` 稳定传递，不泄漏 application 内脏。

### 决策 4：通用工具化先不做“大一统工具”，先建立稳定 contract 读取面

选择：

- 本次不再要求立即实现 `upsertModeArtifact` / `exitMode` 这类过度泛化的新工具。
- 先让 plan-specific 工具通过 `BoundModeToolContractSnapshot` 读取 artifact / exit 合同，消除硬编码重复。
- 后续若要做真正的通用 mode 工具，再基于该 snapshot 单独开 change。

原因：

- 当前 generic tool 方案缺少稳定的 contract 输入面，也没有清楚定义“exit 到哪个 target mode”。
- 直接推进会把不完整的治理语义硬塞进工具层。

### 决策 5：plan workflow 的副作用 owner 收回 application orchestration

选择：

- `enterPlanMode` 只负责 mode transition。
- workflow bootstrap、approval、archive、bridge 生成、reconcile 回归 `application::workflow/*` 与对应 helper。
- `session_plan.rs` 保留 plan artifact owner，但不再成为 workflow side effect 的隐式组合根。

原因：

- workflow 迁移、副作用与 bridge 本就属于 application orchestration，而不是 tool handler。
- 当前逻辑散落在 `session_plan.rs`、`session_use_cases.rs`、`enter_plan_mode.rs`，已经形成多个 owner。

### 决策 6：mode catalog 必须拒绝 duplicate `mode_id`，包括 plugin 对 builtin 的影子覆盖

选择：

- `ModeCatalog` 在构造候选快照时检测 duplicate `mode_id`。
- plugin mode 不允许覆盖 builtin `code` / `plan` / `review`，也不允许与其他 plugin 重名。

原因：

- 扩展 mode contract 后，重复 id 已经不是“展示层小问题”，而是能直接篡改治理语义。
- 静默覆盖会让 bootstrap / reload 结果不可预测，且难以诊断。

### 决策 7：reload 继续遵守 idle-only 合同，不再引入“running turn 用旧快照”的并行语义

选择：

- `AppGovernance.reload()` 继续在存在 running session 时拒绝 reload。
- reload 只在 idle 状态下组装候选治理快照：
  - mode catalog
  - capability surface
  - skill catalog
- 成功时一次提交，失败时完整回滚。

原因：

- 这是现有主 spec 和代码已经建立的治理合同。
- 在这个前提下，不存在“执行中 turn 继续用旧快照、下一 turn 再切新快照”的混合语义；那是另一套模型，不能和 idle-only 同时存在。

## Risks / Trade-offs

- [风险] 去掉 `workflow_binding` 后，change 看起来比最初 proposal 更收敛。
  - Mitigation：这是有意收敛，换来 owner 清晰与可实现性；workflow 绑定本来就已有正式 owner。

- [风险] 引入 `BoundModeToolContractSnapshot` 会扩大 core/tool 上下文字段。
  - Mitigation：只引入 pure-data snapshot，不携带 router、锁、channel 或 application 类型。

- [风险] plan workflow 副作用回收进 application 后，短期改动面横跨 `workflow`、`session_plan`、`session_use_cases`。
  - Mitigation：以“迁 owner 不改语义”为原则，先抽 helper，再移动调用点。

- [风险] duplicate `mode_id` 拒绝会让此前依赖覆盖行为的实验性插件失效。
  - Mitigation：仓库本身不追求向后兼容；这里优先保证治理语义确定性。

## Migration Plan

1. 先更新架构文档和 change/spec 术语，删掉 `workflow_binding` 与 mixed-snapshot 语义。
2. 在 `core` 扩展 `GovernanceModeSpec` 的 artifact / exit / prompt 合同，并增加 duplicate `mode_id` 校验需求。
3. 在 `application` 中显式化 mode compile / governance bind 边界。
4. 为 `ResolvedGovernanceSurface -> AgentPromptSubmission -> ToolContext` 增加 pure-data bound mode contract snapshot。
5. 让 builtin `plan` mode 用新 mode contract 字段表达当前 artifact / exit / prompt 语义。
6. 把 plan workflow 的 bootstrap / approval / bridge / reconcile 副作用收回 workflow/application owner。
7. 重构 reload 路径为统一候选治理快照提交/回滚。
8. 补充 duplicate mode id、workflow compile / reconcile、reload rollback、prompt source tracking 与 tool-contract bridge 测试。

## Resolved Questions

- **workflow phase 绑定放哪里**：放在 workflow compiled artifact，不放在 `GovernanceModeSpec`。
- **duplicate mode id 怎么处理**：一律拒绝；plugin 不允许影子覆盖 builtin mode。
- **reload 是否支持执行中 session 混合版本**：不支持；继续遵守 idle-only reload。
- **generic mode tools 是否纳入本次**：不纳入；本次先建立稳定的 tool contract snapshot。
