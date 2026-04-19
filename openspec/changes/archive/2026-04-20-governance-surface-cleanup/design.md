## Context

当前治理相关逻辑至少散落在以下几条路径：

- `session-runtime::AgentPromptSubmission` 同时承载 scoped router、prompt declarations、resolved limits、injected messages 等多类治理输入，但它只是一个字段集合，而不是显式的治理包络模型。[submit.rs](/D:/GitObjectsOwn/Astrcode/crates/session-runtime/src/turn/submit.rs:36)
- `application::execution::root`、`application::execution::subagent`、`application::session_use_cases` 以不同方式构造这些字段，造成 root / session / child 提交路径的装配方式不一致。[root.rs](/D:/GitObjectsOwn/Astrcode/crates/application/src/execution/root.rs)、[subagent.rs](/D:/GitObjectsOwn/Astrcode/crates/application/src/execution/subagent.rs)、[session_use_cases.rs](/D:/GitObjectsOwn/Astrcode/crates/application/src/session_use_cases.rs)
- child execution contract 由 `application::agent` 中的 helper 直接生成，但静态协作 guidance 又在 `adapter-prompt::WorkflowExamplesContributor` 内硬编码，authoritative 来源分裂。[agent/mod.rs](/D:/GitObjectsOwn/Astrcode/crates/application/src/agent/mod.rs:314)、[workflow_examples.rs](/D:/GitObjectsOwn/Astrcode/crates/adapter-prompt/src/contributors/workflow_examples.rs:17)
- `PromptFactsProvider` 已经是稳定的 prompt 事实入口，但当前治理相关 builtin 事实并未统一从这里注入。[prompt_facts.rs](/D:/GitObjectsOwn/Astrcode/crates/server/src/bootstrap/prompt_facts.rs)
- 三条路径各自独立构建 `CapabilityRouter`：root 从 kernel gateway 计算，subagent 做 allowed_tools 交集，resume 在 `routing.rs` 中独立构建。[root.rs:71](/D:/GitObjectsOwn/Astrcode/crates/application/src/execution/root.rs:71)、[subagent.rs:141](/D:/GitObjectsOwn/Astrcode/crates/application/src/execution/subagent.rs:141)、[routing.rs:571](/D:/GitObjectsOwn/Astrcode/crates/application/src/agent/routing.rs:571)
- `ResolvedExecutionLimitsSnapshot`、`ExecutionControl`、`ForkMode`、`SubmitBusyPolicy` 散落在不同层，没有统一信封。[agent/mod.rs:580](/D:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:580)、[execution_control.rs](/D:/GitObjectsOwn/Astrcode/crates/core/src/execution_control.rs)、[submit.rs:23](/D:/GitObjectsOwn/Astrcode/crates/session-runtime/src/turn/submit.rs:23)
- `PolicyEngine` 已有完整的三态策略框架（Allow/Deny/Ask）和审批流类型，但当前只有 `AllowAllPolicyEngine` 且没有真实消费者，与执行路径完全脱钩。[engine.rs](/D:/GitObjectsOwn/Astrcode/crates/core/src/policy/engine.rs:289)
- `DelegationMetadata`、`SpawnCapabilityGrant`、`AgentCollaborationPolicyContext` 由 `agent/mod.rs` 中的局部 helper 各自拼装。[agent/mod.rs:287](/D:/GitObjectsOwn/Astrcode/crates/application/src/agent/mod.rs:287)、[agent/mod.rs:100](/D:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:100)
- `PromptFactsProvider` 中存在隐式治理联动：vars dict 传递 `agentMaxSubrunDepth` 等参数，`prompt_declaration_is_visible` 用 capability name 做隐式过滤。[prompt_facts.rs:86](/D:/GitObjectsOwn/Astrcode/crates/server/src/bootstrap/prompt_facts.rs:86)、[prompt_facts.rs:200](/D:/GitObjectsOwn/Astrcode/crates/server/src/bootstrap/prompt_facts.rs:200)
- `AppGovernance` 和 `RuntimeCoordinator` 管理运行时治理生命周期，但缺少 mode catalog 的明确接入点。[governance.rs:84](/D:/GitObjectsOwn/Astrcode/crates/application/src/lifecycle/governance.rs:84)、[coordinator.rs:29](/D:/GitObjectsOwn/Astrcode/crates/core/src/runtime/coordinator.rs:29)
- `AgentCollaborationFact` 记录协作审计事件，但缺少与治理包络的上下文关联，导致审计链路无法追溯到治理决策依据。[agent/mod.rs:1129](/D:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1129)

这导致一个问题：治理逻辑已经客观存在，却没有一条清晰的 authoritative assembly path。后续 governance mode system 如果直接叠加，会被迫同时接管 `application` helper、`session-runtime` ad-hoc submission 字段、`adapter-prompt` 静态 guidance、策略引擎接线、能力路由构建、执行限制计算、委派元数据生成、prompt 事实联动等多条路径。

## Goals / Non-Goals

**Goals:**

- 定义统一的 `ResolvedGovernanceSurface` / 等价 execution envelope，作为进入 `session-runtime` 前的标准治理输入。
- 让 root execution、普通 session submit、subagent fresh/resumed launch 统一复用同一治理装配器。
- 统一三条能力路由装配路径（root/subagent/resume），使 capability router 由治理装配器统一生成。
- 把 `ResolvedExecutionLimitsSnapshot`、`ExecutionControl`、`ForkMode`、`SubmitBusyPolicy` 等执行限制收敛为治理包络的一部分。
- 为 `PolicyEngine` 建立与治理包络的接入管线，使策略引擎能消费治理包络，同时本轮保持 `AllowAllPolicyEngine` 默认行为。
- 将 `DelegationMetadata`、`SpawnCapabilityGrant`、`AgentCollaborationPolicyContext` 的生成收口到治理装配路径。
- 把协作 guidance、delegation catalog、child execution contract 的 authoritative 来源收口到治理装配层。
- 显式化 `PromptFactsProvider` 中的隐式治理联动，使 prompt 事实成为治理包络的消费者。
- 让 `AppGovernance` 和 `RuntimeCoordinator` 为后续 mode catalog 接入预留明确入口。
- 让协作审计事实（`AgentCollaborationFact`）能关联治理包络上下文。
- 保持 `adapter-prompt` 的职责回到"渲染已有 prompt declaration / few-shot"，而不是继续承载治理真相。
- 为后续 governance mode system 预留稳定接入点，但本次不实现 mode 本身。

**Non-Goals:**

- 不实现新的 governance mode catalog 或 mode transition。
- 不改变 `run_turn`、tool cycle、streaming path 或 compaction 核心流程。
- 不新增独立 crate。
- 不在本轮实现完整的审批拦截逻辑；只建立策略引擎与治理包络的管线，默认行为保持放行。
- 不在本轮追求完全删除所有 builtin prompt contributor；只收口与治理强相关的 authoritative 逻辑。

## Decisions

### Decision 1：引入统一治理包络，而不是继续扩张 `AgentPromptSubmission`

**选择：**

新增一个显式治理包络类型，例如 `ResolvedGovernanceSurface`，承载：

- scoped capability router
- prompt declarations
- resolved execution limits
- resolved context overrides
- inherited / injected messages
- child delegation metadata（如适用）
- resolved policy context（PolicyEngine 消费用）
- collaboration audit context（协作事实记录用）

`AgentPromptSubmission` 退化为 transport/helper 形状，或直接被该类型替代。

**理由：**

- 当前 `AgentPromptSubmission` 只是字段集合，难以表达"这是已经完成治理装配后的 turn 输入"。
- governance mode system 后续也需要一个统一的 envelope 接入点，本次 cleanup 可以先把底座做对。

**替代方案：**

- 继续往 `AgentPromptSubmission` 塞字段：被拒绝，会让提交 API 更混乱。

### Decision 2：治理装配器放在 `application`，`session-runtime` 只消费结果

**选择：**

- `application` 新增治理装配服务，负责根据入口类型、profile、capability grant、delegation metadata 等解析最终治理包络。
- `session-runtime` 只消费已经解析好的治理包络，不在底层重新做业务级策略判断。

**理由：**

- 这符合仓库中 `application` 是治理入口、`session-runtime` 是单 session 真相面的边界。
- 也符合后续 mode transition / mode compile 最终应在 `application` 完成决策的方向。

**替代方案：**

- 在 `session-runtime` 内做治理装配：被拒绝，会把业务治理下沉到底层。

### Decision 3：把 authoritative governance prompt 迁移到 declaration 装配链路

**选择：**

- 协作 guidance、child contract、delegation-specific builtin blocks 由治理装配器生成 `PromptDeclaration`
- `adapter-prompt` 只负责渲染这些 declaration
- `WorkflowExamplesContributor` 保留非治理 few-shot 内容，治理专属内容迁出

**理由：**

- 当前 prompt declaration 已经是跨边界稳定 contract，适合承载 authoritative governance 事实。
- 这样 mode 系统未来只需要改变 declaration 装配结果，不需要修改 adapter 里的硬编码逻辑。

**替代方案：**

- 让 `adapter-prompt` 继续直接拼 governance 文本：被拒绝，会让 adapter 再次偷渡业务真相。

### Decision 4：fresh / resumed child contract 走同一治理装配路径

**选择：**

- fresh child、resumed child、普通 session prompt submit 共用同一治理装配总入口
- 允许入口参数不同，但输出形状一致

**理由：**

- 现在 fresh/resumed child contract 虽然都在 `application::agent`，但仍是局部 helper，不是正式装配路径。
- 统一后才容易在 mode 系统中替换"哪套 child policy 生效"，而不是分别 patch 多条路径。

**替代方案：**

- 继续保留 fresh/resumed 专属 helper，各自由调用方决定是否使用：被拒绝，容易再次分叉。

### Decision 5：cleanup 以行为等价为目标，不在本轮引入新的产品语义

**选择：**

- 本轮重构以"authoritative 来源收口"和"模块职责清晰"为目标
- execute 默认行为、child contract 语义、协作 guidance 语义尽量保持现状等价

**理由：**

- 这是 governance mode system 的前置，不应该在同一轮里同时引入结构重构和新治理语义。

### Decision 6：CapabilityRouter 构建逻辑统一到治理装配器

**选择：**

- root/subagent/resume 三条路径的 capability router 构建逻辑全部迁入治理装配器
- 治理装配器接收 kernel gateway 的全局能力面和入口参数（如 SpawnCapabilityGrant），统一计算子集
- `execution/root.rs:71-85`、`execution/subagent.rs:141-172`、`agent/routing.rs:571-722` 中的独立构建逻辑被替换

**理由：**

- 三条路径的核心逻辑是"从全局能力面中选取当前 turn 可用的子集"，语义相同但实现分散。
- mode system 后续需要按 mode 改变能力面选择策略，统一后只需修改一处。

**替代方案：**

- 保留三条路径各自构建，只在 mode 系统时统一：被拒绝，会在 mode 实现时同时修改三个文件。

### Decision 7：执行限制类型统一收口为治理包络的字段

**选择：**

- `ResolvedExecutionLimitsSnapshot`、`ExecutionControl`、`ForkMode`、`SubmitBusyPolicy` 在治理装配阶段统一解析
- 治理包络承载完整的执行限制信息，提交路径不再独立计算这些值
- `AgentConfig` 中的治理参数（max_subrun_depth、max_spawn_per_turn 等）作为治理装配器的输入，不直接被消费方读取

**理由：**

- 当前各消费方（root.rs、subagent.rs、session_use_cases.rs、agent/mod.rs）各自从 runtime config 或参数中读取限制值，容易不一致。
- 治理包络作为唯一事实源，消除参数散落导致的不一致风险。

**替代方案：**

- 保持各消费方独立读取 config：被拒绝，mode system 会需要在多处同时修改参数来源。

### Decision 8：PolicyEngine 接入治理包络，但本轮不实现非平凡策略

**选择：**

- `PolicyContext` 从治理包络派生，不再独立组装
- 策略引擎的三个检查点能读取治理包络中的能力面和执行限制
- 本轮保持 `AllowAllPolicyEngine` 作为唯一实现，不引入审批拦截
- 审批流类型（ApprovalRequest/ApprovalResolution/ApprovalPending）的管线建立但默认不触发

**理由：**

- 策略引擎是 mode system 的核心执行检查点，如果本轮不接管线，mode 实现时需要同时做管线接入和策略逻辑。
- 先建立管线但不改变默认行为，风险最小。

**替代方案：**

- 完全跳过策略引擎集成：被拒绝，mode system 的审批能力无法复用已有框架。
- 本轮实现完整审批逻辑：被拒绝，scope 过大，与 cleanup 目标冲突。

### Decision 9：DelegationMetadata 和 SpawnCapabilityGrant 从治理包络派生

**选择：**

- `build_delegation_metadata`、`SpawnCapabilityGrant` 的构建逻辑迁入治理装配器
- `AgentCollaborationPolicyContext` 从治理包络中获取 max_subrun_depth/max_spawn_per_turn
- `enforce_spawn_budget_for_turn` 使用治理包络中的参数

**理由：**

- 委派策略元数据是治理决策的核心输出，由局部 helper 生成意味着治理逻辑分散。
- mode system 后续需要按 mode 改变委派策略，统一后只需修改装配器。

**替代方案：**

- 保留各 helper 独立构建，mode system 时统一：被拒绝，与 Decision 6 同理。

### Decision 10：PromptFactsProvider 退化为治理包络的消费者

**选择：**

- `prompt_declaration_is_visible` 过滤逻辑上移到治理装配层
- `PromptFacts.metadata` 中的治理参数（agentMaxSubrunDepth 等）从治理包络获取，不再通过 vars dict 传递
- `build_profile_context` 中的 approvalMode 与治理包络中的策略配置保持一致

**理由：**

- 当前 `PromptFactsProvider` 既是事实收集器又是隐式治理过滤器，职责不清。
- 显式化后，prompt 事实与能力面/执行限制使用同一治理事实源，消除不一致风险。

**替代方案：**

- 保持 vars dict 隐式传递：被拒绝，string-keyed dict 容易出错，且 mode system 需要更强的类型安全。

### Decision 11：AppGovernance 和 RuntimeCoordinator 预留 mode catalog 接入点

**选择：**

- `GovernanceBuildInput` 增加可选的 mode catalog 参数（本次传 None）
- `AppGovernance.reload()` 编排中预留 mode catalog 替换步骤（本次为空操作）
- `RuntimeCoordinator.replace_runtime_surface` 后续 turn 使用更新后的治理包络

**理由：**

- mode catalog 需要在 bootstrap/reload 阶段装配，如果不预留接入点，mode 实现时需要修改治理生命周期编排。
- 预留但不实现，本轮不增加运行时开销。

**替代方案：**

- mode system 实现时再加：被接受作为备选，但预留接口成本很低，且能减少 mode 系统的改动面。

### Decision 12：协作审计事实关联治理包络上下文

**选择：**

- `AgentCollaborationFact` 增加可选的治理包络标识字段（如 governance_revision 或 envelope_hash）
- `CollaborationFactRecord` 的构建参数从治理包络获取
- 本轮不要求审计事实改变语义，只增加关联能力

**理由：**

- 协作审计是治理闭环的重要环节。如果审计事实无法追溯到治理决策，mode system 的治理验证会缺少关键数据。
- 低成本增加字段，不影响现有审计逻辑。

**替代方案：**

- 不在审计中增加治理上下文：被接受作为备选，但会增加 mode system 的调试难度。

## Risks / Trade-offs

- **[Risk] 收口 authoritative prompt 来源时出现行为漂移**
  → Mitigation：先用等价文本迁移，保留现有回归测试，并增加 root/session/subagent 三入口的一致性测试。

- **[Risk] 新治理包络类型与后续 mode envelope 重叠**
  → Mitigation：本轮命名和字段设计直接按后续可扩展方向做，避免做一次性中间态 DTO。

- **[Risk] application 层装配器过重**
  → Mitigation：装配器只负责治理输入解析与声明生成，不吞并 session-runtime 的执行控制逻辑。

- **[Risk] CapabilityRouter 构建逻辑统一后，各入口的差异化需求丢失**
  → Mitigation：治理装配器支持入口类型参数化（root/subagent/resume），保留必要的差异化计算，只统一计算框架。

- **[Risk] PolicyEngine 管线建立后，后续 mode 系统的审批逻辑可能需要重构管线**
  → Mitigation：本轮只建立最小管线（PolicyContext 从包络派生），不引入复杂的审批流编排，后续可增量扩展。

- **[Risk] PromptFactsProvider 退化后，prompt 事实的可测试性下降**
  → Mitigation：治理装配器本身需要独立的单元测试，PromptFactsProvider 的测试改为验证它正确消费了治理包络。

- **[Trade-off] 短期内会同时存在旧 helper 与新装配路径的迁移代码**
  → 接受：优先保证行为等价和渐进迁移，待 mode system 接入后再删残留 helper。

- **[Trade-off] 预留 mode catalog 接入点可能引入未使用的参数**
  → 接受：参数为 Option 类型，不传时无运行时开销；mode 系统实现时直接填充即可。

## Migration Plan

1. 定义统一治理包络类型与 application 装配服务。
2. 先接 root execution、subagent launch，再接普通 session submit。
3. 将三条 capability router 构建路径统一迁入治理装配器。
4. 把 execution limits、ForkMode、SubmitBusyPolicy 的解析迁入治理装配器。
5. 为 PolicyEngine 建立 PolicyContext 从治理包络派生的管线。
6. 把 child contract 与协作 guidance 的 authoritative 来源迁到治理装配器。
7. 将 DelegationMetadata、SpawnCapabilityGrant、AgentCollaborationPolicyContext 的生成迁入治理装配器。
8. 显式化 PromptFactsProvider 中的治理联动，使其成为治理包络消费者。
9. 在 AppGovernance/RuntimeCoordinator 中预留 mode catalog 接入点。
10. 为协作审计事实增加治理包络上下文关联。
11. 将 `session-runtime` submit 入口收敛为消费治理包络。
12. 清理旧 helper 和分散命名，保留最小兼容桥接直到 mode system 接入。

回滚策略：

- 若装配收口引发行为回归，可暂时保留旧调用路径，让入口回退到旧 helper；因为本轮不触碰 turn engine，回滚范围局限在治理装配层。

## Open Questions

1. `ResolvedGovernanceSurface` 应下沉到 `core` 还是先放在 `application` / `session-runtime` 边界附近？
2. 普通 session prompt submit 是否也需要显式治理包络，还是只为 root/subagent 先收口？
3. `WorkflowExamplesContributor` 中哪些块属于"few-shot 教学"，哪些属于"authoritative governance guidance"，边界是否需要更明确的 contributor 切分？
4. PolicyEngine 的管线建立是否需要同步考虑 MCP 级别的 approval 管线（`adapter-mcp::config::approval`），还是先只关注 application/session-runtime 层？
5. 治理包络是否需要区分"编译时"（mode 编译为包络）和"提交时"（包络传入 session-runtime）两个阶段，还是合并为一个阶段？
6. ForkMode 的选择是否应该由治理包络中的策略决定，还是继续由 SpawnAgentParams 的调用方决定？
7. 协作审计事实的治理上下文关联，是用轻量级标识（如 hash）还是结构化摘要？
