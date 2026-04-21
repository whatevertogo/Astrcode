## Why

Astrcode 当前已经形成 `CapabilitySpec`、`GovernanceModeSpec`、`WorkflowDef` 与 `PromptDeclaration` 多套声明模型，但 compile、bind、orchestrate 的边界还没有统一语言，导致治理编译、插件 mode 注册、reload、一致性与 prompt 注入路径难以收敛。更紧迫的是，`plan` mode 仍然依赖 `upsertSessionPlan` / `exitPlanMode` 这类硬编码工具与 artifact 约定，说明 `GovernanceModeSpec` 的表达能力还不足以支撑真正可插件化的 mode。

## What Changes

- 统一声明式编译骨架，明确 `compile`、`bind`、`orchestrate` 三类职责的边界与命名约束。
- 扩展 `GovernanceModeSpec` 的表达能力，使 mode 可声明 artifact 合同、exit gate、动态 prompt hook 和 workflow 绑定信息，而不再依赖 `plan` 专属硬编码。
- 明确插件声明与消费路径，把 `InitializeResultData.modes`、mode catalog、capability surface 与 governance 编译阶段串成一条一致的 host 注册链路。
- 收敛 mode prompt program 与治理 helper prompt 的来源语义，要求统一沉淀到现有 `PromptPlan` 结果模型，而不是新增平行 prompt IR。
- 补齐 governance reload 的一致性约束，要求 mode catalog、capability surface、skill catalog 的切换满足原子替换或完整回滚。
- 明确 workflow 侧采用轻量 compiled artifact 语义，但不在本次引入为当前规模不必要的索引化数据结构。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `governance-mode-system`: 扩展 mode spec 的声明能力，并要求插件 mode、mode catalog、selector 编译与 reload 一致性共同收敛。
- `mode-capability-compilation`: 明确 selector 求值是 mode compiler 的核心算法，并要求 compile 结果与 child/grant 裁剪边界清晰稳定。
- `mode-prompt-program`: 收敛 mode prompt、治理 helper prompt 与 prompt 结果模型之间的关系，明确来源与注入责任。
- `workflow-phase-orchestration`: 增加轻量 workflow compile/validate 语义，并补充 mode/workflow 绑定边界。
- `governance-reload-surface`: 强化 mode catalog、capability surface、skill catalog 在 reload 时的一致性要求与失败回滚语义。

## Impact

- 影响 `crates/core/src/mode/mod.rs`、`crates/application/src/mode/*`、`crates/application/src/governance_surface/*`、`crates/application/src/workflow/*` 的治理与编排边界。
- 影响 `crates/protocol/src/plugin/handshake.rs` 对 plugin mode 声明的消费约束，以及 `crates/server/src/bootstrap/governance.rs` / `capabilities.rs` 的 reload 路径。
- 影响 builtin `plan` mode 与 `enterPlanMode` / `exitPlanMode` / `upsertSessionPlan` 的通用化设计，但本 change 不直接承诺一次性移除所有现有工具。
- 需要同步更新 `PROJECT_ARCHITECTURE.md` 或相关架构文档，使仓库级架构说明与新的 compile/bind/mode-contract 术语保持一致。
