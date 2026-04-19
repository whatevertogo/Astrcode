## Context

AstrCode 当前已经有几条与 mode 高度相关、但尚未统一收口的执行链路：

- `session-runtime` 在 turn 开始时固定工具面，[runner.rs](/D:/GitObjectsOwn/Astrcode/crates/session-runtime/src/turn/runner.rs:157) 直接从 `gateway.capabilities().tool_definitions()` 取可见工具。
- `PromptFactsProvider` 与 `PromptDeclaration` 已经是稳定的 prompt 注入入口，[request.rs](/D:/GitObjectsOwn/Astrcode/crates/session-runtime/src/turn/request.rs:256) 会基于当前 capability surface 解析 prompt facts。
- `AgentPromptSubmission` 已经具备 turn-scoped execution envelope 雏形，可同时携带 scoped router、prompt declarations、resolved limits 和 inherited messages，[submit.rs](/D:/GitObjectsOwn/Astrcode/crates/session-runtime/src/turn/submit.rs:36)。
- 子 agent 路径已经能生成 capability-aware child contract，[agent/mod.rs](/D:/GitObjectsOwn/Astrcode/crates/application/src/agent/mod.rs:314)，但当前仍是单独硬编码逻辑。
- 协作 guidance 仍以全局固定 prompt block 形式存在，[workflow_examples.rs](/D:/GitObjectsOwn/Astrcode/crates/adapter-prompt/src/contributors/workflow_examples.rs:94)，尚未纳入统一治理配置。
- `PolicyEngine` 已有完整三态策略框架（Allow/Deny/Ask）和审批流类型，但只有 `AllowAllPolicyEngine` 实现，无真实消费者，[engine.rs](/D:/GitObjectsOwn/Astrcode/crates/core/src/policy/engine.rs:289)。
- `CapabilityPromptContributor` 和 `AgentProfileSummaryContributor` 通过 `PromptContext.tool_names` / `capability_specs` 间接感知工具面变化，无需自身了解 mode 概念，[capability_prompt.rs](/D:/GitObjectsOwn/Astrcode/crates/adapter-prompt/src/contributors/capability_prompt.rs)、[agent_profile_summary.rs](/D:/GitObjectsOwn/Astrcode/crates/adapter-prompt/src/contributors/agent_profile_summary.rs)。
- `StorageEventPayload` 已有完整的 tagged event 体系，[event/types.rs](/D:/GitObjectsOwn/Astrcode/crates/core/src/event/types.rs:101)，但无 mode 变更事件。
- `AgentStateProjector` 提供了增量事件投影模式，[agent_state.rs](/D:/GitObjectsOwn/Astrcode/crates/core/src/projection/agent_state.rs)，可作为 mode 投影的扩展参考。
- `Command` enum 已有 `/new`、`/resume`、`/model`、`/compact` 等 slash 命令，[command/mod.rs](/D:/GitObjectsOwn/Astrcode/crates/cli/src/command/mod.rs:14)，可扩展 `/mode`。

经 governance-surface-cleanup 后，上述散落逻辑已被收口为统一治理装配路径，mode system 在此基础上实现。

因此，这次设计的关键不是"再发明一个模式概念"，而是把 mode 提升为统一的执行治理配置，并让它复用 cleanup 后的 envelope、prompt 与 capability surface 事实源。

## Goals / Non-Goals

**Goals:**

- 定义开放式 governance mode 模型，让 builtin 与未来插件 mode 都能通过 catalog 注册。
- 在 turn 边界把当前 mode 编译为 `ResolvedTurnEnvelope`，统一承载 capability router、prompt declarations、execution limits、action policy 与 child policy。
- mode 的能力选择通过 `CapabilitySelector` 从当前 `CapabilitySpec` 投影，支持组合操作。
- mode 的执行限制与用户 `ExecutionControl` 取交集，更严格者生效。
- mode 的 action policies 驱动 `PolicyEngine` 三态裁决，使策略引擎成为实际治理检查点。
- mode 的 prompt program 通过 `PromptDeclaration` 注入标准 prompt 管线。
- 保持 `run_turn` / tool cycle / compaction / streaming 只有一套通用实现，避免 mode 分叉 runtime engine。
- 让 session 的当前 mode 通过 durable event 驱动投影保存，支持恢复、回放与审计。
- 提供 `/mode` slash 命令，支持切换、补全和状态显示。
- 让 child delegation surface 与协作 guidance 受当前 mode 约束。
- 让 `DelegationMetadata`、`SpawnCapabilityGrant` 从 mode child policy 推导。
- 让协作审计事实关联 mode 上下文。
- 为未来插件扩展 mode 留出正式入口，同时限制插件只能扩展治理输入，不能接管 loop 本身。

**Non-Goals:**

- 不新增独立 `mode-system` crate。
- 不引入 `run_plan_turn` / `run_review_turn` / `run_execute_turn` 等多套 loop。
- 不在本轮实现通用 artifact 平台；plan/review 输出契约只预留治理接口，不先做重量级产物系统。
- 不让插件直接替换 `run_turn`、tool cycle、streaming path 或 compaction 算法。
- 不在本轮把所有 builtin prompt contributor 全部删除；首批只把与治理强相关的协作/委派路径收口。

## Decisions

### Decision 1：不新增 crate，沿现有边界分层落地

**选择：**

- `core`：定义 `ModeId`、`GovernanceModeSpec`、`CapabilitySelector`、`ActionPolicies`、`ChildPolicySpec`、`ResolvedTurnEnvelope`
- `application`：提供 builtin / plugin mode catalog、transition validator、envelope compiler
- `session-runtime`：保存当前 mode 投影，并在 submit 边界应用 envelope
- `server`：在 bootstrap / reload 中装配 mode catalog

**理由：**

- 当前仓库已经明确 `core` 承载稳定语义、`application` 承载治理编排、`session-runtime` 承载单 session 真相，不需要再拆一层新的 mode facade。
- 如果现在新增 crate，反而会逼迫它跨边界持有 `CapabilityRouter`、`PromptDeclaration`、session 投影与 plugin 注册等多重职责，迅速形成第二个 orchestration 中心。

**替代方案：**

- 新建 `mode-system` crate：被拒绝。收益主要是"名字更显眼"，代价是边界更差。

### Decision 2：mode 使用开放式 ID + catalog，不使用封闭枚举

**选择：**

- 不使用 `CollaborationMode enum`
- 使用开放式 `ModeId(String)` + `GovernanceModeSpec`
- builtin `execute` / `plan` / `review` 只是 catalog 中的内置条目

**理由：**

- 未来要支持插件自定义 mode，封闭枚举会直接把扩展点焊死。
- 当前真正稳定的不是"只有三个 mode"，而是"所有 mode 都要能编译成同一类治理包络"。

**替代方案：**

- 先用 enum，未来再迁移：被拒绝。迁移成本高，还会污染协议、事件与测试。

### Decision 3：mode 编译为治理包络，而不是直接控制 loop

**选择：**

在 turn 提交时解析当前 mode，编译出：

- `capability_router`（通过 CapabilitySelector）
- `prompt_declarations`（通过 prompt program）
- `execution_limits`（max_steps、ForkMode、SubmitBusyPolicy）
- `action_policies`（供 PolicyEngine 消费）
- `child_policy`

并把这些作为 `ResolvedTurnEnvelope` 传入统一的 turn 执行链路。

**理由：**

- 现有 `AgentPromptSubmission` 已经证明 "turn-scoped envelope" 是天然适配点。
- `session-runtime` 的 turn runner 目前只需要一个已收敛的工具面和 prompt 输入，不需要知道 mode 名称。
- 这样可以让 mode 真正控制执行治理，又不会把 mode 侵入到 tool cycle、compaction、streaming 等内部算法。

**替代方案：**

- 为每种 mode 定制不同的 `agent_loop`：被拒绝。会把治理策略与执行引擎耦合，后续插件 mode 不可控。
- 只生成 prompt，不收口 capability 与 child policy：被拒绝。太弱，无法支撑治理配置目标。

### Decision 4：mode capability 选择严格建立在现有 capability surface 之上

**选择：**

mode 不维护平行工具目录，而是通过 `CapabilitySelector` 从当前 `CapabilitySpec` / capability router 投影能力面。首批支持：

- `Name`：精确匹配工具名
- `Kind`：匹配 CapabilityKind
- `SideEffect`：匹配副作用级别
- `Tag`：匹配元数据标签
- `AllTools`：选择全部工具
- 组合操作：交集（Intersection）、并集（Union）、差集（Difference）

**理由：**

- 与现有 `capability-semantic-model` 要求一致，避免重新长出第二事实源。
- 组合操作使 mode spec 能表达复杂约束（如"所有 Tool 类但排除 External 副作用"）。

**替代方案：**

- 继续只靠工具名字白名单：可实现但过脆，不利于插件扩展。
- 继续把 `side_effect` 当万能选择器：被拒绝，治理语义和资源副作用语义不是一回事。

### Decision 5：session 只保存当前 mode 投影，durable truth 来自事件

**选择：**

- 新增 `ModeChanged` durable event（`StorageEventPayload` 新变体）
- `AgentState` 增加 `mode_id` 字段
- `SessionState` 增加 per-field mutex 缓存当前 mode
- replay 旧会话时默认回退为 builtin `execute`

**理由：**

- 项目架构已经明确 durable truth 优先来自 event log，`SessionState` 只是 projection cache + live control。
- mode 会跨 turn 影响行为，也需要恢复与审计，必须进入事件流。
- 现有 `AgentStateProjector` 的增量 apply 模式提供了清晰的扩展参考。

**替代方案：**

- 直接把 `session_mode` 视为内存真相：被拒绝。与当前架构方向冲突。

### Decision 6：mode transition 由 application 治理，session-runtime 只负责应用

**选择：**

- `application` 负责校验 target mode、entry policy、transition policy
- `session-runtime` 只接收已验证的 transition command，追加 durable event 并更新投影
- `/mode`、UI 快捷键、工具调用都映射到统一应用用例，不在 `session-runtime` 中解析壳命令语法

**理由：**

- 仓库明确规定 slash command 只是输入壳，语义要映射到稳定 server/application contract。
- 这样可以把 mode transition 与未来 approval / governance 策略对齐，而不是在 runtime 内再造一套权限系统。

**替代方案：**

- 直接在 `session-runtime` 里解析 `/mode`：被拒绝，会把输入壳语义沉到底层。

### Decision 7：child session 的初始 mode 由 child policy 推导，而不是简单继承

**选择：**

- 父 mode 不直接把自己的 `mode_id` 原封不动传给 child
- 父 mode 的 `child_policy` 决定 child 是否允许 delegation、默认 child mode、是否允许 child 再委派
- child 的 `SpawnCapabilityGrant` 和 `DelegationMetadata` 从 child policy 推导

**理由：**

- 真正需要传递的是治理策略，而不是标签本身。
- 这能自然兼容 fresh / resumed / restricted child 的既有 contract 语义。
- `SpawnCapabilityGrant` 与 mode capability selector 的交集确保 child 能力面不超过 mode 约束。

**替代方案：**

- 默认子 agent 继承父 mode：太粗糙，无法表达"父在 plan，但子可 execute"之类治理规则。

### Decision 8：协作 guidance 与 child contract 改为消费 governance prompt program

**选择：**

- 保留 `PromptDeclaration` 作为唯一 prompt 注入格式
- 当前 mode 编译生成 prompt program，再映射为 declaration
- `workflow_examples` 与 child execution contract 逐步从硬编码文本改为消费 mode policy
- `CapabilityPromptContributor` 和 `AgentProfileSummaryContributor` 无需感知 mode——它们通过 `PromptContext` 间接响应能力面变化

**理由：**

- 现有 prompt 管线已经稳定，不需要再开旁路。
- `PromptDeclarationContributor` 已能渲染任意 declaration，mode 生成的 declarations 无需特殊处理。
- contributor 的自动响应模式（依赖 tool_names 守卫条件）是最小侵入的实现方式。
- 这也是未来插件自定义编排提示词的最小侵入接入点。

**替代方案：**

- 允许插件直接替换 prompt composer 或注入任意 loop hooks：被拒绝，风险过高。

### Decision 9：mode 编译 action policies 供 PolicyEngine 消费

**选择：**

- mode spec 定义 action policies（允许/拒绝/需审批的能力调用规则）
- envelope 编译时将 action policies 作为 `PolicyEngine` 的配置
- `PolicyContext` 从治理包络派生，不再独立组装
- 本轮 builtin mode 只使用 Allow/Deny（不触发 Ask 审批流）
- 插件 mode 可定义需要审批的 action policies

**理由：**

- `PolicyEngine` 已有完整框架但无消费者。mode 是让它发挥价值的自然时机。
- 通过 action policies 而非直接实现 `PolicyEngine` trait，保持了 mode 与策略引擎的解耦。
- 不需要修改 turn loop，策略检查在现有检查点位置执行。

**替代方案：**

- mode 不连接 PolicyEngine，另建治理检查机制：被拒绝，会制造第二套策略系统。
- 本轮实现完整审批流：被拒绝，scope 过大。

### Decision 10：mode 执行限制与 ExecutionControl 取交集

**选择：**

- mode spec 可指定 max_steps 上限、ForkMode 约束、SubmitBusyPolicy 偏好
- 用户通过 `ExecutionControl` 指定的限制与 mode 限制取交集（更严格者生效）
- mode 未指定的参数使用 runtime config 默认值

**理由：**

- mode 代表治理约束，用户控制代表即时需求，两者应该叠加而非覆盖。
- 这避免了 mode 限制被用户参数绕过，也避免了 mode 限制过度约束用户的合理需求。

**替代方案：**

- mode 限制覆盖用户参数：被拒绝，用户失去即时控制能力。
- 用户参数覆盖 mode 限制：被拒绝，治理约束可被绕过。

### Decision 11：/mode 命令集成到现有 CLI 命令体系

**选择：**

- 在 `Command` enum 增加 `Mode { query: Option<String> }` 变体
- `parse_command` 增加 `"/mode"` arm
- tab 补全从 mode catalog 获取候选
- 命令路由到 application 统一治理入口

**理由：**

- 与 `/model`、`/compact` 等命令遵循相同模式，学习成本低。
- 通过 slash candidates 机制自然支持 tab 补全。

**替代方案：**

- 仅通过工具调用切换 mode：被拒绝，用户无法直接控制。
- 在 session-runtime 中解析命令：被拒绝，违反"命令壳语义上移"原则。

### Decision 12：协作审计事实关联 mode 上下文

**选择：**

- `AgentCollaborationFact` 增加可选的 `mode_id` 字段
- 审计事实记录当前 turn 开始时的 mode（不受 turn 内 mode 变更影响）
- observability 快照包含当前 mode 和变更时间戳

**理由：**

- 协作审计是治理闭环的重要环节。mode 上下文使审计能追溯到治理决策。
- 低成本增加字段，不影响现有审计逻辑。

**替代方案：**

- 不在审计中增加 mode 上下文：被接受作为备选，但增加调试和审计难度。

## Risks / Trade-offs

- **[Risk] mode 选择器语义不够精确，导致 builtin / plugin capability 分类漂移**
  → Mitigation：首批只支持少量稳定 selector，并优先复用现有 `CapabilitySpec` 字段；新增语义时扩展 semantic model，而不是旁路建表。

- **[Risk] 把太多治理逻辑压到 mode，导致 action policy 与 runtime approval 发生重复**
  → Mitigation：mode 只表达"默认治理输入与可见行为边界"，最终高风险动作的批准仍保留给统一治理入口。

- **[Risk] 现有静态协作 guidance 重构后出现行为漂移**
  → Mitigation：先保留 builtin 默认 prompt program，重构时做等价测试，确保 execute mode 下行为近似当前默认行为。

- **[Risk] 插件 mode 破坏稳定性**
  → Mitigation：插件 mode 只能注册 catalog/spec 和 prompt program，不能替换 runtime engine；reload 继续沿现有原子替换 capability surface 的治理链路。

- **[Risk] PolicyEngine 接入后审批流过于复杂**
  → Mitigation：首轮 builtin mode 只使用 Allow/Deny，不触发 Ask 审批。审批流管线存在但不启用。

- **[Risk] /mode 命令在 turn 执行中触发竞态**
  → Mitigation：mode transition next-turn 生效语义确保当前 turn 不受影响；application 层校验在 turn 开始时而非中途执行。

- **[Risk] CapabilitySelector 组合操作性能开销**
  → Mitigation：capability surface 规模有限（通常 <100），组合操作在 turn 边界一次性执行，不影响 tool cycle 性能。

- **[Trade-off] 首轮不做通用 artifact 平台**
  → 接受：先把治理输入、transition 和 turn envelope 站稳，比过早引入 plan/review 通用产物平台更重要。

- **[Trade-off] builtin mode 的 prompt program 初期可能仍是文本块**
  → 接受：先确保管线正确，再逐步将硬编码文本迁移为结构化 prompt program。

## Migration Plan

1. 在 `core` 引入 mode 稳定类型，但默认 builtin `execute` mode 与现有行为等价。
2. 在 `core` 增加 `ModeChanged` 事件载荷和 `CapabilitySelector` 类型。
3. 在 `application` 装配 builtin mode catalog，并实现 envelope compiler（包含 capability selector 编译和 action policy 编译）。
4. 在 `session-runtime` 引入 `ModeChanged` 事件与当前 mode 投影；旧会话回放默认视为 `execute`。
5. 将 turn 提交改为先编译并应用 `ResolvedTurnEnvelope`，但保持现有 `run_turn` 主循环不变。
6. 将 PolicyEngine 的 PolicyContext 改为从治理包络派生。
7. 逐步把协作 guidance 与 child contract 改为消费 mode policy。
8. 在 CLI 增加 `/mode` 命令和 tab 补全。
9. 让 DelegationMetadata 和 SpawnCapabilityGrant 从 mode child policy 推导。
10. 在 bootstrap/reload 中装配 mode catalog。
11. 为协作审计事实增加 mode 上下文。
12. 后续再把插件 mode 接到 bootstrap / reload，不需要回滚 turn engine。

回滚策略：

- 若实现中断或行为不稳定，可临时只保留 builtin `execute` mode，并让 envelope compiler 退化为"当前默认行为的等价编译"。
- 因为 `run_turn` 不改成多实现，回滚只需关闭 mode compile / transition 接线，不需撤销整个 turn 引擎。

## Open Questions

1. mode transition 是否需要与现有 approval / policy engine 做统一结果模型，而不是返回简单文本错误？
2. 插件 mode 的 schema 校验放在 bootstrap 期还是 reload 期统一校验，失败时是否整批拒绝？
3. governance prompt program 是否需要支持"覆盖 builtin block"还是只支持追加/排序？
4. child policy 的默认项是否要区分 root session 与 child session，以避免过度委派链条？
5. CapabilitySelector 的组合操作是否需要支持嵌套（如 `Union(Name("a"), Intersection(Kind(Tool), NotTag("experimental")))`），还是限制为一层？
6. mode 执行限制的 max_steps 是否需要区分"硬上限"（不可超过）和"建议值"（用户可覆盖）？
7. `/mode` 命令是否需要支持 mode 参数化（如 `/mode plan --focus=frontend`），还是首轮只支持 ID 切换？
8. PolicyEngine 的 Ask 审批流在插件 mode 场景下的超时和默认行为是什么？
