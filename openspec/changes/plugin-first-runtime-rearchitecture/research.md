## 调研目标

- 评估 Astrcode 是否应该从当前的 `session-runtime + application + kernel + server bootstrap` 结构，重构为更接近 `pi-mono` 的“最小运行时核心 + plugin-first host + hooks 总线”架构。
- 回答三个核心问题：
  - 现在的 `session-runtime`、`application`、`plugin/sdk` 各自承担了什么职责，哪里已经出现边界漂移。
  - 现有 hooks 方向与 `pi-mono` 的扩展系统，哪些可以直接借鉴，哪些不能直接照搬。
  - 在“不做向后兼容”的前提下，哪种重构方向最符合仓库当前约束。
- 本次调研范围聚焦后端运行时与扩展层，不展开前端 UI 或协议细节重做。

## 当前现状

### 相关代码与模块

- `PROJECT_ARCHITECTURE.md`
  - 当前架构权威文档明确把 `session-runtime` 定义为“单会话执行引擎和事实边界”，并要求它内部同时承载事件溯源层、运行时状态层、外部接口层。
  - 文档还明确要求 `server` 是唯一组合根，`application` 是业务编排层，`plugin` 是宿主侧插件运行时。
- `crates/session-runtime/src/lib.rs`
  - `SessionRuntime` 当前公开了大量“上层宿主能力”而不只是 turn loop：`list_sessions`、`list_session_metas`、`create_session`、`create_child_session`、`observe`、`conversation_snapshot`、`conversation_stream_replay`、`session_child_nodes`、`session_mode_state`、`active_task_snapshot`、`replay_stored_events`、`wait_for_turn_terminal_snapshot` 等。
  - 这说明它已经不是“薄 runtime core”，而是 runtime + session service façade + query/read model 入口的组合。
- `crates/session-runtime/src`
  - 当前共有 79 个源码文件，这个数量级本身就说明它不是“最小 turn 执行内核”，而是一整个宿主系统。
- `crates/application/src/lib.rs`
  - `App` 持有 `governance_surface`、`mode_catalog`、`workflow_orchestrator`、`mcp_service`、`agent_service` 等多个高层组件。
  - `application` 还直接 re-export 了大量 `astrcode_session_runtime`、`astrcode_kernel`、`astrcode_core` 类型，说明它并没有完全退到稳定 host 边界后面。
- `crates/application/src/ports/session_submission.rs`
  - `AppAgentPromptSubmission` 中仍然直接包含 `astrcode_kernel::CapabilityRouter`。
  - 同时存在 `impl From<AppAgentPromptSubmission> for astrcode_session_runtime::AgentPromptSubmission`，说明 `application -> runtime` 之间仍存在具体结构泄漏。
- `crates/server/src/bootstrap/runtime.rs`
  - 当前组合根显式区分并组装多条事实源：`core builtin tools`、`agent tools`、`MCP invokers`、`plugin invokers`、`plugin modes`、`governance surface`、`capability sync`、`runtime coordinator`。
  - 这是一种“server 手工拼接多条特例路径”的模型，而不是 plugin-first 的统一注册表模型。
- `crates/sdk/src/lib.rs`、`crates/sdk/src/hook.rs`
  - 当前 SDK 暴露的核心能力主要是 `ToolHandler`、`HookRegistry` / `PolicyHookChain`、`PluginContext`、`StreamWriter`。
  - 这套能力足够支撑“工具 + 少量策略 hook”，但还不足以表达统一扩展系统里的 provider、resource、prompt、command、shortcut、resource discovery 等贡献面。
- `crates/plugin/src/lib.rs`
  - 当前插件系统的核心仍是“插件进程管理 + JSON-RPC capability bridge + stdio streaming”。
  - 它更像“能力调用基础设施”，还不是一套完整的 plugin host。
- `crates/core/src/lib.rs`
  - 当前 `core` 导出了大约 120 个公开类型与 trait，且 `crates/core/src` 下共有 58 个源码文件。
  - 其中混杂了共享值对象、ports、projection、workflow、mode、plugin registry、session catalog、observability、config 等多类 owner 专属内容。
- `crates/kernel/src/lib.rs`
  - `kernel` 当前主要只是重新导出 `KernelBuilder`、`KernelGateway`、`CapabilityRouter`、`SurfaceManager`、`EventHub` 等聚合/路由对象。
  - 结合 `kernel/src/kernel.rs` 可见它本质上更接近一个 service locator / provider aggregator，而不是拥有独立业务真相的正式架构层。
- `crates/server/src/bootstrap/providers.rs`
  - 当前 `ConfigBackedLlmProvider` 明确拒绝 `provider_kind != openai` 的配置，并在运行时只实例化 `OpenAiProvider`。
  - 这说明 Astrcode 虽然有 `LlmProvider` trait，但当前产品级 provider 抽象仍明显偏薄，尚未形成 plugin-first 的 provider registry。
- 多 agent 协作相关实现
  - `crates/core/src/agent/` 当前已经包含 `SubRunHandle`、`CollaborationExecutor`、`SubAgentExecutor`、`InputQueueProjection` 等协作模型和合同。
  - `crates/application/src/agent/` 当前包含 `AgentOrchestrationService`、`launch_subagent`、`subrun_event_context` 等编排逻辑。
  - `crates/session-runtime/src/turn/` 与 `crates/session-runtime/src/state/` 当前包含 `subrun_events`、`persist_subrun_finished_event`、`cancel_subruns_for_turn`、`input_queue`、`query/subrun` 等持久化与查询路径。
  - 这说明 Astrcode 还有一条 `pi-mono` 本身没有的“多 agent 协作”主线，而且它现在横跨 `core`、`application`、`session-runtime` 三层。
- 既有 hooks 相关提案
  - `openspec/changes/archive/2026-04-21-introduce-hooks-platform-crate/proposal.md`
  - `openspec/changes/archive/2026-04-21-introduce-hooks-platform-crate/specs/lifecycle-hooks-platform/spec.md`
  - `openspec/changes/archive/2026-04-21-extract-governance-prompt-hooks/proposal.md`
  - 这些历史提案已经把方向指向“独立 hooks 平台 + 复用 `PromptDeclaration` 注入链路”，说明仓库内部其实已经出现了朝 plugin-first / hook-first 演进的前置设计。
- `pi-mono` 参考实现
  - `D:/GitObjectsOwn/pi-mono/packages/agent/src/agent.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/agent/src/types.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/extensions/loader.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/extensions/runner.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/agent-session.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/resource-loader.ts`
  - 这些文件对应了 `pi` 的三层结构：最小 `agent-core`、上层 `AgentSession`、再上层的 extension/resource host。

### 相关接口与能力

- Astrcode 当前对外扩展面仍以“能力调用”为中心：
  - `CapabilitySpec` / `CapabilityWireDescriptor`
  - plugin capability invoker
  - `core::hook` 中较窄的 lifecycle hook 事件
- `crates/core/src/hook.rs` 当前事件面只覆盖：
  - `PreToolUse`
  - `PostToolUse`
  - `PostToolUseFailure`
  - `PreCompact`
  - `PostCompact`
  - 这离一个完整的运行时扩展总线还有明显距离。
- `pi-mono` 当前扩展面明显更宽：
  - `ExtensionAPI` 支持 `on`、`registerTool`、`registerCommand`、`registerShortcut`、`registerProvider`、`sendMessage`、`sendUserMessage`、`appendEntry`、`setModel`、`setThinkingLevel` 等。
  - `ExtensionRunner` 已经内建多种事件分发语义：顺序分发、可取消、可拦截、可修改、管道式、短路式。
- `pi` 的 `AgentOptions` 非常薄，只保留：
  - `convertToLlm`
  - `transformContext`
  - `streamFn`
  - `getApiKey`
  - `beforeToolCall`
  - `afterToolCall`
  - `toolExecution`
  - `transport`
  - `steeringMode`
  - `followUpMode`
  - 这证明 `pi` 的核心 runtime 只承担 loop、provider 调用、tool dispatch、event 流等最小职责。
- `pi` 的 `AgentSession` 与 `ResourceLoader` 则承担更上层的事情：
  - session 持久化
  - compaction
  - bash 执行
  - extension 绑定
  - skill / prompt / theme / context file 发现
  - 这和 Astrcode 当前“很多 host 逻辑被塞进 `session-runtime` 与 `application`”的状态形成鲜明对比。

### 相关数据与模型

- Astrcode 当前关键模型
  - `crates/core/src/capability.rs`：`CapabilitySpec`
  - `crates/protocol/src/capability/descriptors.rs`：`CapabilityWireDescriptor` 直接复用 `CapabilitySpec`
  - `crates/core/src/plugin/manifest.rs`：`PluginManifest`
  - `crates/core/src/ports.rs`：`PromptDeclaration`、`PromptGovernanceContext`
  - `crates/core/src/hook.rs`：`HookInput`、`HookOutcome`、`ToolHookContext`、`CompactionHookContext`
  - `crates/session-runtime/src/lib.rs`：`SessionObserveSnapshot`、`TurnTerminalSnapshot`、`ProjectedTurnOutcome`、`AgentPromptSubmission`
- 从这些模型可以看出：
  - DTO / 协议层本身并不是完全失控，问题核心在于“哪些模型被错误放进了 core，哪些模型被错误提升成跨 crate 合同”。
  - 换句话说，问题不是“DTO 数量多”，而是“owner 专属 DTO 被 core 化了”。
- `pi-mono` 当前关键模型
  - `AgentOptions` / `AgentState`
  - `BeforeToolCallContext` / `BeforeToolCallResult`
  - `AfterToolCallContext` / `AfterToolCallResult`
  - `AgentEvent`
  - `ExtensionRuntime` 里的 pending provider registration
  - `ResourceExtensionPaths`
- `pi` 的模型特点是：
  - runtime 模型小而稳定
  - host / extension 模型在 runtime 之上增量扩展
  - handler 结果大多是“拦截 / 增补 / 修改 / 取消”这种 effect 风格

### 测试、约束与边界

- 当前硬约束
  - `PROJECT_ARCHITECTURE.md` 是权威架构文档。本次目标与它当前对 `session-runtime` 的定义存在明显冲突，因此不能只改代码不改文档。
  - schema 与仓库约定都强调：会话持久化优先基于事件日志，而不是隐式内存状态。
  - DTO / 协议层必须保持纯数据，不得把运行时内脏泄漏到外部边界。
  - `server` 仍应保持唯一组合根。
  - 本项目明确不要求向后兼容。
- 当前已有验证与守卫
  - `node scripts/check-crate-boundaries.mjs` 是依赖边界守卫。
  - `crates/sdk/src/tests.rs`、`crates/protocol/src/plugin/tests.rs` 已覆盖一部分 SDK / 协议现状。
  - 但这次调研没有逐个盘点所有 runtime 相关测试，只能确认“现有测试面存在”，不能把它当作完整迁移保障。
- 关键边界张力
  - 如果要做成 pi 风格的最小 runtime core，就必须把“事件日志 / 回放 / projection / session catalog”与“live turn runtime”解耦。
  - 但项目又要求事件日志优先，因此更合理的做法不是丢掉事件日志，而是把它上移到 host-session 层，而不是继续让它定义 runtime core 的边界。
  - 同时，plugin-first 并不等于“一切都改成外部子进程”。热路径上的 hooks、builtin tools、provider 适配仍需要进程内 builtin plugin 形态。

## 必须删除或归零的旧内容

这轮额外勘察确认：本次 change 不只是“新增 3 个 crate”，而是需要在迁移完成后让一批旧边界彻底消失。否则仓库会长期处于“双边界并存”的半重构状态。

### 整 crate 删除

- `crates/application/**`
  - 当前共有 80 个源码文件。
  - 该 crate 里需要整体消失的旧子域包括：
    - `src/agent/**`
    - `src/execution/root.rs`
    - `src/execution/subagent.rs`
    - `src/governance_surface/**`
    - `src/mode/**`
    - `src/workflow/**`
    - `src/mcp/mod.rs`
    - `src/ports/**`
    - `src/lib.rs` 对 `AgentOrchestrationService`、`GovernanceSurfaceAssembler`、`ModeCatalog`、`WorkflowOrchestrator` 的公开导出
- `crates/kernel/**`
  - 当前共有 16 个源码文件。
  - 需要整体消失的旧边界包括：
    - `src/kernel.rs`
    - `src/gateway/mod.rs`
    - `src/registry/**`
    - `src/agent_surface.rs`
    - `src/agent_tree/**`
    - `src/surface/mod.rs`
- `crates/session-runtime/**`
  - 当前共有 80 个源码文件。
  - 迁移完成后旧 crate 应整体删除；核心目录包括：
    - `src/turn/**`
    - `src/state/**`
    - `src/query/**`
    - `src/catalog/**`
    - `src/context_window/**`
    - `src/observe/**`
    - `src/command/**`
    - `src/actor/**`
- 旧 `crates/plugin/**` 边界
  - 当前共有 `loader.rs`、`process.rs`、`peer.rs`、`supervisor.rs`、`worker.rs`、`capability_router.rs` 等宿主实现。
  - 这些实现不是全部废弃，而是迁入 `plugin-host` 后，旧 `crates/plugin` 作为独立正式边界应删除。

### `core` 中必须删掉的旧共享面

- `crates/core/src/projection/**`
- `crates/core/src/mode/**`
- `crates/core/src/config.rs`
- `crates/core/src/observability.rs`
- `crates/core/src/session_plan.rs`
- `crates/core/src/store.rs`
- `crates/core/src/composer.rs`
- `crates/core/src/plugin/registry.rs`
- `crates/core/src/session_catalog.rs`
- `crates/core/src/runtime/traits.rs`
- `crates/core/src/plugin/manifest.rs` 中旧 `PluginManifest`
- `crates/core/src/hook.rs` 中旧 `HookInput`、`HookOutcome`
- `crates/core/src/agent/lineage.rs` 中 `SubRunHandle`
- `crates/core/src/agent/input_queue.rs` 中 `InputQueueProjection`
- `crates/core/src/lib.rs` 中对应的旧 re-export
  - 包括旧 `PluginRegistry`、`PluginManifest`、`PluginHealth`、`PluginState`、`PluginType`
  - 包括旧 `SessionCatalogEvent`
  - 包括 `session_plan`、`observability`、`store`、`composer` 相关 re-export

这些内容的问题不只是“代码老”，而是它们把 owner 专属模型错误提升成了共享依赖面。

### 必须消失的旧特判装配路径

- `crates/server/src/bootstrap/runtime.rs`
  - 里面当前手工拼接 builtin tools、agent tools、MCP invokers、plugin invokers、governance、mode、workflow、capability sync。
- `crates/server/src/bootstrap/providers.rs`
  - 里面当前保留 `provider_kind == openai` 的硬编码选择路径。
- `crates/server/src/bootstrap/plugins.rs`
- `crates/server/src/bootstrap/mcp.rs`
- `crates/server/src/bootstrap/governance.rs`
- `crates/server/src/bootstrap/capabilities.rs`
- `crates/server/src/bootstrap/composer_skills.rs`
- `crates/server/src/bootstrap/prompt_facts.rs`
- `crates/server/src/bootstrap/watch.rs`
- `crates/server/src/bootstrap/deps.rs`
- `crates/server/src/bootstrap/runtime_coordinator.rs`

这里不要求这些文件名全部删除，但要求其中承载的“组合根内业务特判逻辑”必须消失，不能原样搬到新架构里。

### 多 agent 协作相关的旧跨层实现

- `crates/application/src/agent/mod.rs` 中 `AgentOrchestrationService`
- `crates/application/src/agent/routing.rs`
- `crates/application/src/agent/routing/child_send.rs`
- `crates/application/src/agent/routing/parent_delivery.rs`
- `crates/application/src/agent/observe.rs`
- `crates/application/src/agent/terminal.rs`
- `crates/application/src/agent/wake.rs`
- `crates/application/src/execution/subagent.rs`
- `crates/session-runtime/src/turn/finalize.rs` 中 subrun finished 持久化路径
- `crates/session-runtime/src/turn/interrupt.rs` 中 cancel subruns for turn 路径
- `crates/session-runtime/src/query/subrun.rs`
- `crates/session-runtime/src/state/input_queue.rs`
- `crates/kernel/src/agent_tree/**`
- `crates/kernel/src/agent_surface.rs`

这些内容最终不能再以“core + application + kernel + session-runtime 四处分摊”的形式存在，而是要么进入 `host-session`，要么只留下最小 child-turn 执行合同在 `agent-runtime`。

## 关键发现

### 发现 1

- 事实：`session-runtime` 当前已经同时拥有 session catalog、query/read model、conversation replay、child lineage、mode state、event replay、turn terminal wait 等大量宿主级接口。
- 证据：`crates/session-runtime/src/lib.rs`、`crates/session-runtime/src` 共 79 个文件
- 影响：它无法继续被视为“最小 agent runtime core”；如果保留现状，只是在外围加 plugin / hook，最终只会得到一个“大核心 + 扩展糖衣”的系统。
- 可复用点：其中与单次 turn loop、流式执行、工具调度直接相关的部分仍可保留为新 runtime core 的素材；查询、目录、回放等能力则更适合上移。

### 发现 2

- 事实：`application` 仍直接暴露并消费大量 `session-runtime` 与 `kernel` 具体结构；`AppAgentPromptSubmission` 甚至直接包含 `CapabilityRouter`，随后再转换为 runtime 的具体提交结构。
- 证据：`crates/application/src/lib.rs`、`crates/application/src/ports/session_submission.rs`
- 影响：这说明 `application` 没有成为稳定 host use-case boundary，而是继续充当“知道太多 runtime 内部结构的编排层”。
- 可复用点：`governance_surface`、`profile resolution`、`McpService`、`WorkflowOrchestrator` 等能力仍可保留，但应重挂到更清晰的 host 层，而不是继续作为 runtime 边界的一部分。

### 发现 3

- 事实：`server` 当前显式区分 `core builtin tools`、`agent tools`、`MCP invokers`、`plugin invokers`、`plugin modes`、`capability sync` 等多套装配路径。
- 证据：`crates/server/src/bootstrap/runtime.rs`
- 影响：这说明系统现在不是“统一扩展面 + 统一 active snapshot”，而是“server 组合根手工缝合多条事实源”。
- 可复用点：现有 `bootstrap_plugins_with_skill_root`、`CapabilitySurfaceSync`、`ToolSearchIndex`、plugin registry 等基础设施，可以作为未来 `plugin-host` 的实现基础，而不必全部重写。

### 发现 4

- 事实：Astrcode 当前 SDK / plugin 更偏向“工具与 capability 调用基础设施”，扩展贡献面明显窄于 `pi`。
- 证据：`crates/sdk/src/lib.rs`、`crates/sdk/src/hook.rs`、`crates/plugin/src/lib.rs`
- 影响：如果目标是“其他一切都通过 plugin 提供”，现有 SDK 和 plugin manifest 不足以直接承载 provider、resource、prompt、workflow overlay、command、hot reload 一致性等能力。
- 可复用点：现有 capability router、JSON-RPC transport、plugin supervisor、manifest 解析都可以继续作为“外部 plugin 执行后端”；缺的是更高层的统一贡献协议和 host registry。

### 发现 5

- 事实：仓库内部已经有明确的 hooks 前置设计，主张把 hooks 升格为独立平台，并通过 `PromptDeclaration` 进入既有 prompt 组装链路，而不是再造一套平行 prompt 系统。
- 证据：
  - `openspec/changes/archive/2026-04-21-introduce-hooks-platform-crate/proposal.md`
  - `openspec/changes/archive/2026-04-21-introduce-hooks-platform-crate/specs/lifecycle-hooks-platform/spec.md`
  - `openspec/changes/archive/2026-04-21-extract-governance-prompt-hooks/proposal.md`
- 影响：如果现在要做 plugin-first runtime 重构，hooks 不应该再被当成“另一个子系统”，而应该直接成为统一扩展总线。
- 可复用点：历史 hooks 提案里的 event/effect 思路、builtin/external shared registry、prompt hook 复用 `PromptDeclaration` 的约束，都可以直接延续。

### 发现 6

- 事实：`pi-mono` 已经验证了一种清晰分层：
  - `agent-core` 只保留最小 runtime 注入点
  - `AgentSession` 承担 session host 逻辑
  - `ExtensionRunner` 与 `ResourceLoader` 承担扩展与资源发现
- 证据：
  - `D:/GitObjectsOwn/pi-mono/packages/agent/src/agent.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/agent/src/types.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/extensions/loader.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/extensions/runner.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/agent-session.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/resource-loader.ts`
- 影响：Astrcode 想变成“核心最小化，一切可扩展”，不是缺一个 hooks trait，而是需要重新划分 crate 边界与 owner。
- 可复用点：可以直接借鉴 `pi` 的晚绑定扩展运行时、queued provider registration、事件分发语义、resource discovery 分层，但不必照搬 `pi` 的全部产品能力。

### 发现 6.1

- 事实：`pi-mono` 的 `Agent` 只感知 `sessionId`、tool hooks、上下文快照和 prompt/continue 执行；真正持有 `SessionManager`、`ResourceLoader`、`sendUserMessage`、`appendEntry`、`setSessionName` 等 session owner 行为的是 `AgentSession`。
- 证据：
  - `D:/GitObjectsOwn/pi-mono/packages/agent/src/agent.ts`
  - `D:/GitObjectsOwn/pi-mono/packages/coding-agent/src/core/agent-session.ts`
- 影响：这说明 `pi` 的真正可借鉴点不是“它已经有多 agent”，而是“它天然把 agent 执行面和 session owner 行为拆开了”。
- 可复用点：Astrcode 可以沿用这个 `session-as-agent-owner` 思路，把 `agent-runtime` 做成只执行某个 session/turn 的最小内核，把 `host-session` 做成持有 session 真相、资源、扩展动作和多 agent 协作 durable truth 的 owner。

### 发现 7

- 事实：`core` 当前过大，不只是“共享语义层”，而是混杂了共享值对象、mega ports、projection、workflow、mode、plugin registry、session catalog、observability、config 等多种 owner 专属模型。
- 证据：`crates/core/src/lib.rs`、`crates/core/src/ports.rs`、`crates/core/src/projection`、`crates/core/src/workflow.rs`、`crates/core/src/mode`、`crates/core/src/plugin/registry.rs`、`crates/core/src/session_catalog.rs`、`crates/core/src` 共 58 个文件
- 影响：任何想复用“最小 runtime”或“共享消息模型”的场景，都会被迫拉入一大批并不属于 core 的依赖。
- 可复用点：`ids`、`LlmMessage`、`ToolCallRequest`、`CapabilitySpec`、`PromptDeclaration` 这类真正共享的值对象仍适合留在 core。

### 发现 8

- 事实：`kernel` 当前更像 service locator，而不是正式的业务 owner 边界。
- 证据：`crates/kernel/src/lib.rs` 主要只 re-export `KernelBuilder`、`KernelGateway`、`CapabilityRouter`、`SurfaceManager`、`EventHub`；`kernel/src/kernel.rs` 主要做 provider 注入与校验
- 影响：它不值得继续保留为长期 crate 边界，更适合拆回 `agent-runtime`、`host-session` 与 `plugin-host`。
- 可复用点：其中的 router/gateway 思路可以转化为 `plugin-host` 的统一注册表与 active snapshot 组装逻辑。

### 发现 9

- 事实：当前产品级 LLM provider 抽象仍偏薄，运行时实际上只支持 OpenAI 家族 provider。
- 证据：`crates/server/src/bootstrap/providers.rs` 中 `ConfigBackedLlmProvider` 对 `provider_kind != openai` 直接报错；`crates/adapter-llm` 当前只实现 OpenAI 家族 provider
- 影响：如果要做成 plugin-first 架构，provider 贡献和选择逻辑不能继续硬编码在 server/bootstrap 中，必须进入统一的 provider contribution / registry 体系。
- 可复用点：`adapter-llm` 现有 OpenAI 兼容实现可以保留，但需要改成“后端实现之一”，而不是“唯一正式路径”。

### 发现 10

- 事实：Astrcode 的多 agent 协作不是附属功能，而是已经落进 durable truth、query/read model、父子 lineage、turn 中断与结果投递的正式主链；但这套能力当前分散在 `core`、`application`、`session-runtime` 三个 crate。
- 证据：
  - `crates/core/src/agent/lineage.rs`
  - `crates/core/src/agent/executor.rs`
  - `crates/core/src/agent/input_queue.rs`
  - `crates/application/src/agent/mod.rs`
  - `crates/application/src/execution/subagent.rs`
  - `crates/session-runtime/src/turn/finalize.rs`
  - `crates/session-runtime/src/turn/interrupt.rs`
  - `crates/session-runtime/src/query/subrun.rs`
- 影响：如果在新架构里不先明确 collaboration owner，迁移时一定会把同一条事实链继续拆散到 runtime、host、core 三边，最后既不像 `pi-mono` 的最小 runtime，也无法维持 Astrcode 现有的子 agent 可恢复性。
- 可复用点：可以借鉴 `pi-mono` “新的 agent 行为应该通过 session owner 和扩展动作进入系统”的思路，但 Astrcode 仍必须保留“一个 session 即一个 agent”的 durable collaboration truth，因此更适合把协作真相上移到 `host-session`，把协作入口做成 plugin/tool/command surface，把最小执行合同留在 `agent-runtime`。

## 建议的具体迁移映射

### `agent-runtime` 应接收的实现

- 迁入：
  - `session-runtime/turn/*` 中与 loop、llm cycle、tool cycle、continuation cycle、loop control、streaming 直接相关的模块
  - 取消语义、runtime 事件分发、tool dispatch、hook dispatch
- 新结构建议：
  - `runtime.rs`
  - `loop.rs`
  - `types.rs`
  - `tool_dispatch.rs`
  - `hook_dispatch.rs`
  - `stream.rs`
  - `cancel.rs`

### `host-session` 应接收的实现

- 迁入：
  - session catalog
  - event log writer / recovery / replay
  - query/read model
  - branch/fork
  - compaction orchestration
  - observe / conversation snapshot
  - `SubRunHandle`、父子 session lineage、sub-run finished/cancel 事件
  - `InputQueueProjection`、跨 session 输入投递、子 agent 结果回传
  - session/query 公共 surface

### `core` 的建议去留

- 继续保留：
  - `ids.rs`
  - `action.rs` 中的基础消息模型
  - `capability.rs`
  - 极少数真正跨 owner 共享的 prompt/hook 语义
- 应迁出或删除：
  - `projection/` -> `host-session`
  - `workflow.rs` -> `host-session`
  - `session_plan.rs` -> `host-session`
  - `session_catalog.rs` -> `host-session`
  - `ports.rs` -> 按 owner 拆散
  - `plugin/` -> `plugin-host`
  - `mode/` -> `plugin-host` 或 builtin plugin
  - `observability.rs` -> `host-session`
  - `runtime/traits.rs` -> 删除或迁回 owner 专属合同

## 可选方案比较

| 方案 | 适用前提 | 优点 | 风险/代价 | 结论 |
| --- | --- | --- | --- | --- |
| A：保留当前 `session-runtime/application/kernel` 主体，只在外层继续叠加 hooks / plugins | 只想低成本追加扩展点，不追求彻底边界重建 | 改动相对小，短期内更容易落地 | 保留大核心；`application` 与 `session-runtime` 的边界泄漏不会消失；最终很难达到 `pi` 那种“核心最小化” | 不推荐 |
| B：按 plugin-first 方向重构为“最小 runtime core + host-session + plugin-host + hooks 总线” | 接受 breaking change，并愿意同步更新架构文档、spec 与 crate 边界 | 最符合用户目标；能统一 builtin / external 行为；与既有 hooks 方向一致 | 影响范围大，需要分阶段迁移 `session-runtime`、`application`、`server`、`plugin/sdk` | 推荐 |
| C：只把 tools / MCP / discovery plugin 化，保留 governance / workflow / session truth 在旧结构里 | 只想减少部分 bootstrap 特判 | 可以较快减少一部分 server 组装分支 | 仍会保留“两套事实源”：核心内置逻辑一套，plugin 扩展一套；hooks 很难成为统一总线 | 不推荐作为最终形态 |

## 结论

- 推荐方向：采用方案 B，把 Astrcode 重构为“`agent-runtime` + `host-session` + `plugin-host` + hooks 总线”的 plugin-first 架构。
- 理由：
  - 这是唯一真正符合“不要向后兼容，只要完整良好的实现”和“其他一切通过 plugin 提供”的方向。
  - 当前 `session-runtime`、`application`、`server bootstrap` 的职责都明显偏大，只靠继续补 hooks 无法解决。
  - 既有 hooks 提案与 `pi-mono` 的分层思路并不冲突，反而可以自然合流。
  - Astrcode 现有的多 agent 协作必须继续存在，但它应该以“一个 session 即一个 agent”的原则收敛到 `host-session`，而不是继续散落在 `core`、`application` 和 monolith runtime 中。
- 已决策方向：
  - 新建 `agent-runtime` 与 `host-session` 等 crate，再迁移旧实现；不在原有大 crate 内继续原地抽丝剥茧。
  - 删除 `application` crate，不保留长期兼容 façade；原 use-case 语义重新分配到 `host-session`、`plugin-host` 与 `server` 的新边界上。
  - 删除 `kernel` crate，不再保留 provider/tool/resource 的中间门面。
  - 统一 plugin descriptor 扩展到完整贡献面，至少覆盖 `tools`、`hooks`、`providers`、`resources`、`commands`、`themes`、`prompts`、`skills`。
  - 收缩 `core`，不再把 owner 专属 DTO 与 mega ports 留在共享层。
- 暂不采用的方案：
  - 保留当前大核心，只在外围继续堆 plugin / hook。
  - 只 plugin 化工具与 MCP，但把 workflow / governance / session truth 继续硬编码留在旧核心里。

## 未决问题

- builtin plugin 与 external plugin 的执行模型、失败语义、reload 顺序如何统一定义。
- provider contribution 的最终注册协议如何设计，才能既覆盖现有 OpenAI 家族实现，也允许后续引入新的 provider 后端而不再改 server bootstrap。
