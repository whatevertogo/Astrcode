## Why

AstrCode 当前与执行治理相关的逻辑分散在多个不同行为层：静态协作 guidance 写在 `adapter-prompt`，child execution contract 写在 `application` 的 agent 模块里，turn-scoped router 与 prompt declarations 又通过 `session-runtime::AgentPromptSubmission` 以临时包络形式传递。同时，策略引擎框架（`PolicyEngine`）已定义但未接入实际执行路径，能力路由器（`CapabilityRouter`）在三条入口路径中各自构建，执行限制参数（`ResolvedExecutionLimitsSnapshot`、`ExecutionControl`、`ForkMode`、`SubmitBusyPolicy`）散落在不同层，委派策略元数据（`DelegationMetadata`、`SpawnCapabilityGrant`）由局部 helper 各自拼装，prompt 事实注入中存在隐式治理联动（vars dict 传递治理参数、隐式 capability name 过滤），启动与运行时治理装配（`AppGovernance`、`RuntimeCoordinator`）缺少 mode catalog 接入点，协作审计事实（`AgentCollaborationFact`）缺少治理上下文关联。

这个结构虽然可工作，但职责边界不够清晰，也让即将落地的 governance mode system 缺少一个稳定、统一的治理装配入口。

现在先做这轮 cleanup，是为了在不改变 runtime engine 的前提下，把散落的治理输入收口成一条清晰的数据流。否则后续 mode 系统要么继续叠加在现有混乱接线上，要么被迫与当前实现并存，长期会更难维护。

## What Changes

- 新增统一的 governance surface / execution envelope 装配路径，收口 turn-scoped capability router、prompt declarations、child contract 与其他治理输入。
- 让 root execution、普通 session prompt submit、subagent launch 与 child resume 等入口复用同一套治理装配逻辑，而不是各自拼装 `AgentPromptSubmission`。
- 把 builtin 协作 guidance 与 child execution contract 的 authoritative 来源从分散硬编码迁移到统一治理装配层，`adapter-prompt` 仅负责渲染 `PromptDeclaration`。
- 将三条能力路由装配路径（root/subagent/resume）统一到治理装配器，消除各自独立构建 `CapabilityRouter` 的重复逻辑。
- 把 `ResolvedExecutionLimitsSnapshot`、`ExecutionControl`、`ForkMode`、`SubmitBusyPolicy` 等执行限制与控制输入收敛为治理包络的一部分。
- 为 `PolicyEngine` 建立与治理包络的接入管线，使策略引擎的三态裁决能基于治理包络做出，同时保持 `AllowAllPolicyEngine` 作为默认实现。
- 将 `DelegationMetadata`、`SpawnCapabilityGrant`、`AgentCollaborationPolicyContext` 等委派策略元数据收口到治理装配路径。
- 将 `PromptFactsProvider` 中的隐式治理联动（vars dict 参数传递、capability name 过滤）显式化，使 prompt 事实成为治理包络的消费者而非独立治理装配器。
- 在 `AppGovernance` 和 `RuntimeCoordinator` 层为后续 mode catalog 接入预留明确入口。
- 让协作审计事实（`AgentCollaborationFact`）能关联治理包络上下文，确保审计链路可追溯。
- 整理与治理相关的命名和模块分布，减少 "同一概念在不同 crate 用不同形状表达" 的情况，为后续 governance mode system 提供稳定前置。
- 保持现有默认用户行为尽量等价；本次不引入新的 mode 能力，也不改写 `run_turn` 主循环。

## Capabilities

### New Capabilities
- `governance-surface-assembly`: 定义统一的治理装配入口，要求所有 turn / delegation 入口先解析治理包络，再进入 `session-runtime`。同时覆盖 bootstrap/runtime 治理生命周期边界的清晰化和 mode catalog 接入预留。
- `capability-router-assembly`: 统一 root/subagent/resume 三条路径的 capability router 构建，使能力面收敛逻辑经过治理装配器。
- `execution-limits-control`: 收敛 `ResolvedExecutionLimitsSnapshot`、`ExecutionControl`、`ForkMode`、`SubmitBusyPolicy`、`AgentConfig` 治理参数到治理包络。
- `policy-engine-integration`: 为 `PolicyEngine` 建立与治理包络的接入管线，确保策略引擎能消费治理包络做出裁决。
- `delegation-policy-surface`: 收口 `DelegationMetadata`、`SpawnCapabilityGrant`、`AgentCollaborationPolicyContext` 的生成到治理装配路径。
- `prompt-facts-governance-linkage`: 显式化 `PromptFactsProvider` 中的隐式治理联动，使 prompt 事实成为治理包络的消费者。

### Modified Capabilities
- `agent-tool-governance`: 协作 guidance 必须来自 authoritative governance surface，而不是散落在 adapter 层的静态硬编码。
- `agent-delegation-surface`: child delegation catalog 与 execution contract 必须通过同一治理装配路径生成，同时协作审计事实必须能关联治理包络上下文。

## Impact

- `crates/core` 或 `crates/application` 中会新增统一的治理包络类型与装配接口。
- `crates/application` 的 root/subagent/session 提交流程会改为复用同一治理装配器。
- `crates/application/src/execution/subagent.rs` 的 `resolve_child_execution_limits` 逻辑会迁入治理装配器。
- `crates/application/src/agent/routing.rs` 的 resume 路径 scoped router 构建会迁入治理装配器。
- `crates/application/src/agent/mod.rs` 的 `build_delegation_metadata` 和 spawn budget enforcement 会改为消费治理包络参数。
- `crates/session-runtime` 的 `AgentPromptSubmission` / submit API 形状会被收敛，减少 ad-hoc 字段扩散。
- `crates/core/src/policy/engine.rs` 的 `PolicyContext` 会从治理包络派生，策略引擎会获得治理包络感知能力。
- `crates/server/src/bootstrap/prompt_facts.rs` 的隐式治理联动会显式化，`PromptFactsProvider` 会退化为治理包络的消费者。
- `crates/server/src/bootstrap/governance.rs` 和 `crates/application/src/lifecycle/governance.rs` 会增加 mode catalog 接入预留。
- `crates/adapter-prompt` 的 `WorkflowExamplesContributor` 等治理相关逻辑会被瘦身，authoritative 治理事实转为来自上游 `PromptDeclaration`。
- 用户可见影响应保持最小：本次主要是收口结构、清理职责，为后续 governance mode system 提供实现前置。
