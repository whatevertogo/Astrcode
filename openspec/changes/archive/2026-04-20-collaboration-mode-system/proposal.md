## Why

AstrCode 目前缺少一套显式的执行治理配置模型。现有"模式"相关语义分散在工具可见性、prompt guidance、子 agent capability grant、profile 与运行时策略之间，导致系统只能用零散提示词和局部白名单近似表达"先规划再执行""限制委派""只允许审查"等行为，既不利于统一治理，也无法为未来的插件自定义 agent 编排、提示词与能力边界提供稳定扩展点。

同时，策略引擎（`PolicyEngine`）已定义三态裁决和审批流框架但未接入实际执行路径，能力路由器（`CapabilityRouter`）在 root/subagent/resume 三条路径中各自构建，执行限制参数散落无统一信封，委派策略元数据由局部 helper 拼装，prompt 事实中存在隐式治理联动，启动与运行时治理缺少 mode 接入点，协作审计事实缺少 mode 上下文。如果继续把 mode 当成临时 prompt 技巧处理，后续插件扩展会被迫绕过既有边界，重新长出第二套 orchestration 真相。

现在推进这项变更，是因为仓库已经具备统一 capability surface、`PromptDeclaration` 注入链路、child execution contract、基于事件日志的 session/runtime 架构、以及经 cleanup 收口的统一治理装配路径基础。在此基础上引入 mode 系统是自然且低风险的。

## What Changes

- 引入面向执行治理的 mode 系统，把 mode 定义为可编译的治理配置，而不是仅仅是协作阶段标签。
- 新增开放式 `mode id` / mode catalog / turn-envelope 编译链路，在 turn 边界把当前 mode 解析为可执行包络，统一收口 capability surface、prompt program、action policy、execution limits 与 child policy。
- mode 的能力选择通过 `CapabilitySelector` 从当前 `CapabilitySpec` / capability router 编译出 scoped `CapabilityRouter`，支持基于 name、kind、side_effect、tag 的投影和组合操作。
- mode 编译的执行限制（max_steps、ForkMode、SubmitBusyPolicy、AgentConfig 治理参数）与用户指定的 `ExecutionControl` 取交集。
- mode 编译的 action policies 驱动 `PolicyEngine` 的三态裁决，使策略引擎从悬空框架变为实际消费治理包络的检查点。
- mode 编译的 prompt program 生成 `PromptDeclaration`，通过现有注入路径进入 prompt 组装，控制 contributor 行为。
- `PromptFactsProvider` 的 metadata 和 declaration 过滤与 mode 编译的 envelope 保持一致。
- 新增 `/mode` slash 命令，支持 mode 切换、tab 补全、状态显示，通过统一 application 治理入口校验。
- 让 session 持有当前 mode 的 durable 投影（`ModeChanged` 事件），所有切换经统一治理入口校验，在下一 turn 生效。
- mode catalog 在 bootstrap 阶段装配，reload 时与能力面原子替换，插件 mode 走同一注册路径。
- 让内置 mode 与未来插件 mode 走同一条注册和编译路径；插件可扩展提示词、能力选择和委派策略，但不能直接替换 runtime engine。
- 收口现有静态协作 guidance 与 child execution contract，让其受当前 mode 的 governance program 约束。
- `DelegationMetadata`、`SpawnCapabilityGrant` 从 mode child policy 推导，协作审计事实关联 mode 上下文。
- **BREAKING** mode 的内部建模不再使用封闭枚举假设；实现将改为面向开放 catalog 的稳定 ID + spec 结构。

## Capabilities

### New Capabilities
- `governance-mode-system`: 定义 mode catalog、mode transition、session 当前 mode 投影（事件驱动）、turn 边界的治理包络编译与应用、bootstrap/reload 集成、协作审计 mode 上下文、mode 可观测性。
- `mode-capability-compilation`: mode 通过 CapabilitySelector 编译 scoped CapabilityRouter，支持组合选择器，child capability 从 parent mode child policy 推导。
- `mode-execution-policy`: mode 编译执行限制（max_steps、ForkMode、SubmitBusyPolicy）和 AgentConfig 治理参数覆盖，与 ExecutionControl 取交集。
- `mode-policy-engine`: mode 编译 action policies 驱动 PolicyEngine 三态裁决，PolicyContext 从治理包络派生，context strategy 受 mode 影响。
- `mode-prompt-program`: mode 编译 prompt program 生成 PromptDeclaration，通过标准路径注入，控制 contributor 行为，PromptFactsProvider 与 envelope 一致。
- `mode-command-surface`: `/mode` slash 命令、tab 补全、状态显示、transition 拒绝反馈、统一 application 治理入口。

### Modified Capabilities
- `agent-tool-governance`: 协作工具 guidance 从静态 builtin 规则升级为由当前 governance mode 驱动的 action policy / prompt program。Contributor 自动反映 mode 能力面。
- `agent-delegation-surface`: child delegation catalog 与 child execution contract 体现当前 governance mode 的 child policy。DelegationMetadata、SpawnCapabilityGrant 从 mode child policy 推导。

## Impact

- `crates/core`: 新增 mode 相关稳定类型（ModeId、GovernanceModeSpec、CapabilitySelector、ActionPolicies、ChildPolicySpec、ResolvedTurnEnvelope）和 `ModeChanged` 事件载荷。
- `crates/application`: 新增 builtin/plugin mode catalog、transition 校验、envelope compiler、`/mode` 命令处理用例。
- `crates/session-runtime`: 新增当前 mode 的 durable 投影（AgentState 扩展、SessionState per-field mutex）、mode transition 命令入口、submit 边界的 envelope 应用。
- `crates/cli`: 新增 `Command::Mode` 变体、`/mode` 命令解析和 tab 补全。
- `crates/server`: 在 bootstrap / reload 链路中装配 mode catalog，与能力面替换保持原子性。
- `crates/adapter-prompt` 与现有 child contract 生成路径：从静态 guidance 转为消费 mode 编译结果。
- `crates/core/src/policy`: PolicyEngine 通过 mode action policies 获得真实消费者。
- 用户可见影响：可显式切换执行治理模式，mode 稳定影响工具可见性、委派约束、执行限制和提示词。
- 开发者可见影响：实现必须遵守"mode 扩展 governance input，不扩展 runtime engine"的边界；本次不新增独立 crate，也不引入多套 turn loop。
