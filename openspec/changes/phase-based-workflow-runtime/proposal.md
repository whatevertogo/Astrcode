## Why

当前正式工作流的真相分散在 mode、tool、`application`、`session-runtime` 与 conversation/read-model 多层：`plan` 相关审批与 prompt 注入被硬编码在提交入口，plan->execute 依赖隐式 prompt 暗示衔接，`session-runtime` 又同时维护手写 live 状态与事件投影状态，导致 turn 生命周期、状态边界和后续扩展点越来越不清晰。现在继续叠加新的 mode 或 workflow，只会放大这些结构性问题，因此需要先把“mode 负责什么、workflow 负责什么、runtime 真相在哪里”一次理顺。

## What Changes

- 引入一层显式的 **workflow orchestration**：用 `workflow -> phase -> transition -> bridge` 模型承载跨 turn、跨 mode 的正式工作流，把 `plan -> execute` 落成第一个宏工作流，而不是继续在 `submit_prompt_with_options()` 中写死 plan 特判。
- 把 **phase** 定义为 workflow 的核心执行单元：每个 phase 绑定一个 `mode_id`，并声明自己的角色、artifact 规则、用户信号解释、退出 gate 和 phase bridge；mode 继续负责治理 envelope，而不是直接承担完整业务流程。
- 引入 **TurnCoordinator** 显式收口 turn 生命周期：把 `accept → prepare → run → persist → finalize → deferred_compact` 从多模块分段拼装收为单一协调器的生命周期方法，让 interrupt/fork 也走协调器；使用 generation counter 防护 interrupt/resubmit 竞态，确保 stale finalize 不覆盖新 turn 控制状态。
- 引入 **typed TurnTerminalKind** 和 **TurnProjection**：消除 turn 终态的字符串约定，让 `wait_for_turn_terminal_snapshot()` 等待投影终态而不是扫描事件做启发式判断。
- 引入 **PostLlmDecisionPolicy**：统一 agent loop 中"LLM 返回无工具输出后下一步做什么"的决策，消除 `continuation_cycle` / `loop_control` / `step` 三处散落逻辑的隐式耦合。
- 重构 `application` 的 session 提交流程：由 workflow orchestrator 统一处理 prompt 注入、审批解释、phase 迁移与 artifact bridge，替换当前 plan-specific 的 if/else 分支。
- 重构 `session-runtime` 的状态组织：把 turn runtime 生命周期、压缩状态和事件投影注册表从散落字段中收敛成内聚模型，display `Phase` 完全由事件投影驱动不再被运行时直接变异，减少重复 shadow state，并让 query/control 读取路径依赖 authoritative projection。
- **封死 application-runtime 边界**：移除 `session-runtime` 对低层 execution helper 的 re-export，让 `application` 只通过稳定公开 API 面消费 runtime 能力。
- 明确 `canonical session plan` 与 `execution task` 仍是两套真相：允许 workflow 在 phase 切换时建立显式 bridge，但不允许 `taskWrite` 直接篡改 plan artifact 或其审批状态。
- 补齐或同步仓库级架构权威文档：当前项目约束要求 `PROJECT_ARCHITECTURE.md` 为最高参考，但仓库内未找到该文件；本次变更需要显式补齐该权威说明，或者在交付前更新现有等价架构文档并把引用统一。

### Non-Goals

- 不在本次变更中一次性引入所有新 workflow；首个落地目标仅限 `plan -> execute`，第二个 workflow 只作为验证抽象的后续增量。
- 不把 `taskWrite`、conversation transcript、tool metadata surface 混成同一份 durable truth；现有分层仍保留，只重构它们之间的编排关系。
- 不替换现有通用 turn loop、tool cycle、streaming/compaction 算法；本次聚焦于边界收敛、生命周期建模与 workflow 编排。
- 不要求第一阶段立即改掉所有现有 plan 工具名或前端 surface 名称；可以保留兼容 facade，再逐步内收实现。

## Capabilities

### New Capabilities
- `workflow-phase-orchestration`: 定义正式 workflow、phase、transition、approval signal、artifact bridge 与活跃实例的统一编排合同。

### Modified Capabilities
- `application-use-cases`: `application` 负责 workflow 编排、phase 迁移与 prompt/approval 分发，不再在提交入口硬编码 plan 专属流程。
- `session-runtime`: `session-runtime` 的内部状态模型需要从散落字段收敛为清晰的 turn runtime 生命周期与 authoritative projection 管线。
- `session-runtime-subdomain-boundaries`: `state`、`turn`、`query` 子域边界需要围绕新的 runtime lifecycle 与 projection registry 重新收敛。
- `execution-task-tracking`: execute phase 可以消费 approved plan 的 bridge 上下文，但 task durable truth 仍必须与 canonical plan 严格分层。

## Impact

- 受影响代码：`crates/application/src/session_use_cases.rs`、`crates/application/src/session_plan.rs`、mode/prompt 装配路径、`crates/session-runtime/src/state/*`、`crates/session-runtime/src/turn/*`、plan/task 相关 tool 集成与对应测试。
- 受影响系统：session 提交流程、审批与 artifact 生命周期、runtime recovery/checkpoint、turn control/query read model、plan->execute handoff 语义。
- 依赖与边界影响：需要在 `core` 或新的轻量共享层定义 workflow/phase 协议，但 `application` 仍是编排入口，`session-runtime` 不反向依赖 `application`。
- 迁移策略：第一阶段保留 `enterPlanMode` / `upsertSessionPlan` / `exitPlanMode` 与现有 metadata schema，对外 surface 尽量稳定，内部逐步改为委托新的 workflow orchestrator 与 runtime lifecycle 模型。
- 回滚策略：若 workflow orchestrator 路径不稳定，可临时切回现有 plan-specialized 提交流程，同时保留 runtime projection 重构中已经证明等价的低风险收敛改动。
