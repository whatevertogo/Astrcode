## Why

Astrcode 当前已经形成 `CapabilitySpec`、`GovernanceModeSpec`、`WorkflowDef` 与 `PromptDeclaration` 多套声明模型，但 compile、bind、orchestrate 的边界还没有统一语言，导致治理编译、插件 mode 注册、reload 一致性与 prompt 注入路径难以收敛。

更具体地说，当前方案同时存在三类问题：

- mode contract 想承载更多语义，但边界不清，容易把 workflow 真相和工具执行细节一起塞回 mode。
- workflow phase 与 mode 的绑定已经有正式 owner（`WorkflowPhaseDef.mode_id`），却缺少显式的 validate/compile 语义，导致 phase 迁移、副作用与 bridge 逻辑继续散落。
- reload 已经有“能力面失败则回滚”的雏形，但 mode catalog、capability surface、skill catalog 还没有被当作一个统一治理快照来提交。

这次 change 的目标是把这些边界收干净，而不是继续做一个过度扩张的“超级 DSL”。

## What Changes

- 统一声明式治理链路里的 `compile`、`bind`、`orchestrate` 三类职责与命名约束。
- 扩展 `GovernanceModeSpec` 的表达能力，使 mode 可声明 artifact 合同、exit gate 与动态 prompt hook；不再把 workflow phase 绑定反向塞进 mode spec。
- 明确 workflow compiled artifact 是 phase -> mode 绑定的唯一 owner；同一个 `mode_id` 可以被多个 phase 复用。
- 为工具执行引入纯数据的 bound mode contract snapshot，让需要 artifact / exit 语义的工具通过稳定上下文消费 contract，而不是依赖 application 内部类型或自行猜测 mode 语义。
- 明确插件声明与消费路径，把 `InitializeResultData.modes`、mode catalog、capability surface 与治理编译阶段串成一致的 host 注册链路，并补齐 duplicate `mode_id` 拒绝策略。
- 收敛 mode prompt program 与治理 helper prompt 的来源语义，要求统一沉淀到现有 `PromptPlan` 结果模型，而不是新增平行 prompt IR。
- 补齐 governance reload 的一致性约束，要求 mode catalog、capability surface、skill catalog 在无活跃 session 的前提下以同一候选治理快照切换或完整回滚。
- 把 plan workflow 的 bootstrap / approval / bridge / reconcile 副作用收回到 application 的 workflow orchestration，而不是继续散落在 tool handler 和 session-specific if/else 中。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `governance-mode-system`: 扩展 mode spec 的声明能力，补齐 compile / bind 边界，并为工具执行增加纯数据 contract 投影视图。
- `mode-capability-compilation`: 明确 selector 求值是 mode compiler 的核心算法，并要求 compile 结果与 child/grant 裁剪边界清晰稳定。
- `mode-prompt-program`: 收敛 mode prompt、治理 helper prompt 与 prompt 结果模型之间的关系，明确来源与注入责任。
- `workflow-phase-orchestration`: 增加轻量 workflow compile/validate 语义，并明确 phase -> mode 绑定由 workflow artifact 持有。
- `governance-reload-surface`: 强化 mode catalog、capability surface、skill catalog 在 reload 时的一致性要求与失败回滚语义。

## Impact

- 影响 `crates/core/src/mode/mod.rs`、`crates/application/src/mode/*`、`crates/application/src/governance_surface/*`、`crates/application/src/workflow/*` 的治理与编排边界。
- 影响 `crates/core/src/tool.rs`、`crates/session-runtime/src/turn/submit.rs`、`crates/kernel/src/registry/tool.rs`，以承载稳定的 bound mode contract snapshot。
- 影响 `crates/application/src/session_plan.rs`、`crates/application/src/session_use_cases.rs` 与 builtin `plan` mode 的职责拆分，使 workflow 副作用回归 application orchestration owner。
- 影响 `crates/protocol/src/plugin/handshake.rs` 对 plugin mode 声明的消费约束，以及 `crates/server/src/bootstrap/governance.rs` / `capabilities.rs` 的 reload 路径。
- 需要同步更新 `PROJECT_ARCHITECTURE.md` 或相关架构文档，使仓库级架构说明与新的 compile / bind / workflow-owner / governance-snapshot 术语保持一致。
