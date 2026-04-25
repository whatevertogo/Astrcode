## 背景与现状

当前架构与本次目标存在直接冲突，且冲突点不适合继续通过“补一层抽象”解决。

- `session-runtime` 同时承担了 turn 执行、事件日志、投影、会话目录、branch/fork、恢复、查询等多类职责，已经不是“runtime core”，而是“runtime + host + read model”的混合体。
- `application` 仍然暴露并消费 `session-runtime` / `kernel` 的内部事实，说明它没有形成稳定的上层边界，只是中间编排层。
- `kernel` 作为 provider/tool/resource 聚合门面，在当前结构里承担了过多“为了分层而分层”的职责，既没有形成独立产品边界，也让依赖方向更绕。
- `server` 仍手工拼接 builtin tools、plugin、MCP、governance、workflow、mode catalog 等多条装配路径，导致“核心行为”和“扩展行为”存在并行事实源。
- 当前 hooks 仍偏窄，旧 `core::HookInput` / `HookOutcome` 只能表达 tool/compact 一类场景，无法成为统一扩展总线。

`PROJECT_ARCHITECTURE.md` 目前仍把 `session-runtime`、`application`、`kernel` 定义为长期正式边界，这与本 change 的目标不一致。因此本次实现前，必须先更新 `PROJECT_ARCHITECTURE.md`，把新的 crate 边界和依赖方向提升为仓库级权威约束。

这次设计明确采用“无向后兼容、直接面向最终形态”的策略：

- 不保留 `application` 兼容 façade。
- 不保留 `kernel` 过渡壳层。
- 不把旧 `session-runtime` 继续瘦身为长期边界。
- 不维持“核心特判 + plugin 补充”的双轨实现。

## 设计目标

- 建立最小 `agent-runtime`，只负责单 turn 执行、provider 调用、tool dispatch、hook dispatch、流式输出与取消。
- 建立 `host-session`，统一承接会话 durable truth、事件日志、恢复、branch/fork、read model、模型选择、输入入口与执行面组装。
- 延续“一个 session 即一个 agent”的协作原则，把多 agent 协作中的父子 session、sub-run lineage、input queue、结果投递与跨 turn 取消统一收敛到 `host-session`。
- 建立 `plugin-host`，统一承接 builtin / external plugin 的发现、校验、加载、active snapshot、reload 与资源发现。
- 将 hooks 平台升级为唯一扩展总线，覆盖 runtime、host、resource discovery 的正式事件目录。
- 删除 `application` 与 `kernel`，让边界回到真正稳定且可解释的 owner 上。
- 让 `server` 只保留组合根职责，不再手工维护多套产品事实源。
- 让实现直接面向最终目标形态，不为旧 API、旧 crate、旧装配路径保留长期兼容层。

## 非目标

- 不逐字复刻 `pi-mono` 的产品矩阵；本次借鉴的是“最小核心 + 扩展优先 + 统一注册表”的架构方法。
- 不在本次 change 内重做前端 UI 模型，只做后端边界变化所需的最小适配。
- 不保留旧 `application` / `kernel` / `session-runtime` 的双写、双读或双装配长期过渡层。
- 不把所有 builtin 能力都强行做成外部进程；热路径 builtin plugin 允许进程内执行。

## 方案概览

目标形态采用五层结构：

1. `core`
   - 只保留真正跨 owner 共享的值对象、消息模型、稳定枚举和极少数公共合同。
   - 典型内容包括：`ids`、`LlmMessage` / tool call 相关消息模型、`CapabilitySpec`、最小 prompt 声明模型、hooks 事件键与 effect kind。
   - 不保留 owner 专属 DTO，如 plugin descriptor 家族、执行面 DTO、会话快照、恢复模型、projection、workflow/mode、plugin registry、配置持久化 ports。
   - `core` 的规则不是“纯数据都能放进来”，而是“只有多个 owner 共同消费且语义稳定的数据才允许进入 core”。

2. `agent-runtime`
   - 只负责单 turn / 单 agent 的 live 执行。
   - 输入是 `agent-runtime` 自己公开的 `AgentRuntimeExecutionSurface`。
   - 负责 `context -> before_agent_start -> before_provider_request -> provider stream -> tool_call/tool_result -> turn_end` 这条执行链。
   - 不负责 session 目录、事件日志、resource discovery、settings、workflow、catalog、branch/fork。

3. `host-session`
   - 负责会话 durable truth 与 host use-case。
   - 维护事件日志、恢复、投影、查询、branch/fork、compact、模型选择、输入入口、turn 创建与 `AgentRuntimeExecutionSurface` 组装。
   - 维护多 agent 协作的 durable truth：父子 session 关系、`SubRunHandle`、`InputQueueProjection`、结果投递、子运行取消和 lineage。
   - 它是“runtime 的宿主”，不是“又一个 runtime”。

4. `plugin-host`
   - 负责 builtin / external plugin 的统一发现、描述、校验、候选快照、active snapshot、reload、资源发现。
   - 输出统一 `PluginActiveSnapshot`，供 `host-session` 和 `agent-runtime` 消费。
   - builtin plugin 与 external plugin 共用同一套 descriptor 和 hook/tool/provider/resource 注册面，只在执行后端上区分。

5. `server`
   - 只做组合根。
   - 组装 `host-session`、`agent-runtime`、`plugin-host`、各 adapter 与协议层。
   - 不再承载治理、workflow、plugin 贡献合并、mode catalog 计算等业务真相。

DTO 的归属原则也同步调整：

- `core` 只保留共享语义值对象，不再收纳“只因为是 struct 就进 core”的模型。
- `agent-runtime` 自己拥有执行面、provider/tool 流程输入输出、runtime hook payload/report。
- `host-session` 自己拥有 durable snapshot、recovery checkpoint、projection/read model、session/query 结果。
- `plugin-host` 自己拥有 plugin descriptor、active snapshot、resource descriptor、hook registration descriptor。
- `protocol` 只保留真正跨进程或跨网络传输的线协议模型，不承载宿主内部 DTO。

最终 crate 方向如下：

```text
adapter-* ───────────────┐
                         ├──> plugin-host ──┐
storage / protocol ──────┘                  │
                                            │
core <──────────── agent-runtime <──────────┤
  ^                    ^                    │
  |                    |                    │
  └──────────── host-session <──────────────┘
                        ^
                        |
                      server
```

其中：

- `application` 删除。
- `kernel` 删除。
- 旧 `session-runtime` 拆分后删除。
- 当前 `plugin` crate 的进程管理、stdio JSON-RPC、supervisor、worker 协议实现迁入新的 `plugin-host` 边界。

建议的第一版模块布局如下。

### `agent-runtime` 建议结构

```text
crates/agent-runtime/
├── src/
│   ├── lib.rs
│   ├── runtime.rs
│   ├── loop.rs
│   ├── types.rs
│   ├── tool_dispatch.rs
│   ├── hook_dispatch.rs
│   ├── stream.rs
│   └── cancel.rs
```

- `runtime.rs`：对外唯一执行入口，如 `execute_turn`
- `loop.rs`：单次 turn 的主循环
- `types.rs`：`TurnInput`、`TurnOutput`、`AgentRuntimeExecutionSurface`
- `tool_dispatch.rs`：工具调度与 tool batch 语义
- `hook_dispatch.rs`：runtime owner 的 hook 触发与 effect 解释
- `stream.rs`：provider 流式增量处理
- `cancel.rs`：取消、中断和超时传播

### `host-session` 建议结构

```text
crates/host-session/
├── src/
│   ├── lib.rs
│   ├── host.rs
│   ├── catalog.rs
│   ├── event_log.rs
│   ├── recovery.rs
│   ├── collaboration.rs
│   ├── input_queue.rs
│   ├── projection/
│   ├── query/
│   ├── branch.rs
│   ├── fork.rs
│   ├── compaction.rs
│   ├── execution_surface.rs
│   └── model_selection.rs
```

- `host.rs`：对外 host use-case surface
- `catalog.rs`：session 目录与元信息
- `event_log.rs` / `recovery.rs`：durable truth、恢复、回放
- `collaboration.rs`：父子 session / sub-run 协作编排、结果投递、取消传播
- `input_queue.rs`：session 级输入队列和子 agent 投递队列
- `projection/` / `query/`：read model 与查询结果
- `branch.rs` / `fork.rs`：lineage 和会话分叉
- `compaction.rs`：压缩与 `session_before_compact`
- `execution_surface.rs`：组装 runtime 输入
- `model_selection.rs`：模型选择和 `model_select`

### 新旧模块迁移映射

| 当前位置 | 迁移目标 | 原因 |
| --- | --- | --- |
| `session-runtime/turn/*` 中与 loop、llm/tool cycle、streaming 直接相关的模块 | `agent-runtime` | 属于最小执行内核 |
| `session-runtime` 中的 catalog、query、replay、observe、branch/fork、projection | `host-session` | 属于宿主 durable truth 与 read model |
| `core/src/agent/*` 中的 `SubRunHandle`、`InputQueueProjection`、协作执行合同 | `host-session` owner bridge | 属于跨 turn durable collaboration truth |
| `core/src/agent/*` 中嵌入 `ChildSessionNotification` / `StorageEventPayload` 的 `ChildAgentRef`、`ChildSessionNode`、`ChildSessionLineageKind` | 暂留 `core` | 这些类型当前是 durable event DTO 的序列化组成部分，迁出会造成 `core -> host-session` 反向依赖或重复 wire schema |
| `application/src/agent/*` 与 `application/src/execution/subagent.rs` | `host-session` | 属于父子 session 编排和 child session 启动 |
| `session-runtime/src/turn/subrun_*`、`state/input_queue.rs`、`query/subrun.rs` | `host-session` + `agent-runtime` 最小执行合同 | 持久化/read model 在 host，实际 turn 执行仍在 runtime |
| `plugin` 中的 loader / process / peer / supervisor / worker 协议 | `plugin-host` | 属于统一插件宿主 |
| `kernel` 的 gateway / router / surface 聚合 | `plugin-host` + `host-session` + `agent-runtime` | 不再保留独立 service-locator 边界 |
| `application` 的 use-case façade | `host-session` / `plugin-host` / `server` | 删除穿透层 |
| `core::ports` | 按 owner 拆散 | 不再保留 mega ports |
| `core::projection` / `workflow` / `session_catalog` | `host-session` 或删除 | 不属于共享语义层 |
| `core::mode` | `plugin-host` + `core` 共享合同 | 治理 DSL / builtin mode owner 逻辑迁往 plugin-host；`ModeId`、durable mode-change event DTO、tool-contract snapshot 在协议/事件 DTO 拆分前暂留 core |

## 必须删除的旧实现清单

迁移完成后的目标不是“新边界可用”，而是“旧边界消失”。以下内容应视为本次 change 的正式删除范围。

### 1. 整 crate 删除

- `crates/application/**`
  - 删除整个 crate，而不是只保留一个空 façade。
  - 其中包括 `agent`、`execution`、`governance_surface`、`mode`、`workflow`、`mcp`、`ports`、`terminal_queries` 等子域。
- `crates/kernel/**`
  - 删除整个 crate，不保留 `KernelGateway`、`CapabilityRouter`、`KernelBuilder`、`SurfaceManager` 之类的长期壳层。
- `crates/session-runtime/**`
  - 删除 monolith `session-runtime` crate，迁移后不保留同名长期边界。
- 旧 `crates/plugin/**`
  - 在 `plugin-host` 建立完成后，删除旧 `plugin` crate 作为正式边界；其中可复用的进程管理和 transport 实现迁入新 crate，而不是继续并存。

### 2. `core` 中必须清零的旧共享面

- `crates/core/src/projection/**`
- `crates/core/src/mode/**` 中的治理 DSL / builtin mode owner 逻辑（`ModeId` 与 tool/event wire contract 暂留）
- `crates/core/src/config.rs` 中的 owner-only 配置持久化 / 解析逻辑（runtime config 共享合同暂留）
- `crates/core/src/observability.rs` 中的 owner-only collector / summary 逻辑（wire metrics snapshot 暂留）
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
- `crates/core/src/lib.rs` 中对应旧 re-export
  - 包括 `PluginRegistry` / `PluginManifest` / `PluginHealth` / `PluginState` / `PluginType`
  - 包括 `SessionCatalogEvent`
  - 包括 `session_plan` / `observability` / `store` / `composer` 相关旧导出

判定标准不是“文件是否还在”，而是 `core` 不得再暴露这些 owner 专属模型和合同。

### 3. `application` 中必须迁出后删除的旧编排实现

- `crates/application/src/agent/**`
  - 包括 `AgentOrchestrationService`、child/parent routing、observe、wake、terminal 等协作编排实现。
- `crates/application/src/execution/root.rs`
- `crates/application/src/execution/subagent.rs`
- `crates/application/src/governance_surface/**`
- `crates/application/src/mode/**`
- `crates/application/src/workflow/**`
- `crates/application/src/mcp/mod.rs`
- `crates/application/src/composer/mod.rs`
- `crates/application/src/ports/**`

这些内容要么迁入 `host-session`，要么迁入 `plugin-host`，要么回到 server 组合根；不能再由 `application` 继续承载。

### 4. 协作主线里必须删除的旧跨层真相

- `crates/kernel/src/agent_tree/**`
- `crates/kernel/src/agent_surface.rs`
- `crates/session-runtime/src/query/subrun.rs`
- `crates/session-runtime/src/state/input_queue.rs`
- `crates/session-runtime/src/turn/finalize.rs` 中 subrun 完成持久化路径
- `crates/session-runtime/src/turn/interrupt.rs` 中 subrun 取消传播路径
- `crates/application/src/agent/mod.rs` 中 `SubAgentExecutor` / `CollaborationExecutor` 的旧实现承载点

这些旧实现删除后的唯一真相位置是：

- durable collaboration truth -> `host-session`
- child-session live execution contract -> `agent-runtime`
- collaboration surface exposure -> `plugin-host`

### 5. server 组合根中必须消失的旧特判逻辑

- `crates/server/src/bootstrap/runtime.rs` 中 builtin tools / agent tools / MCP / plugin / governance / workflow / mode 的并列装配特判
- `crates/server/src/bootstrap/providers.rs` 中 `provider_kind == openai` 的硬编码 provider 选择路径
- `crates/server/src/bootstrap/plugins.rs` 中旧 plugin boundary 装配逻辑
- `crates/server/src/bootstrap/mcp.rs` 中独立旁路装配逻辑
- `crates/server/src/bootstrap/governance.rs` 中旧治理面特判逻辑
- `crates/server/src/bootstrap/capabilities.rs` 中旧 capability sync 主线路径
- `crates/server/src/bootstrap/composer_skills.rs` 中技能/命令旁路装配逻辑
- `crates/server/src/bootstrap/prompt_facts.rs` 中旧 prompt facts 旁路解析与注入逻辑
- `crates/server/src/bootstrap/watch.rs` 中旧 profile watch 旁路装配逻辑
- `crates/server/src/bootstrap/deps.rs` 中围绕旧组合根的依赖打包壳层
- `crates/server/src/bootstrap/runtime_coordinator.rs` 中旧运行时协调壳层

这里的目标不是机械删文件，而是删掉“组合根内业务特判”这类旧事实源。

### `adapter-llm` 的新位置

- `adapter-llm` 保留为 provider backend 实现层，而不是 runtime 核心的一部分。
- `agent-runtime` 只依赖抽象的 stream surface，不知道 OpenAI、DeepSeek、Ollama 等 provider 差异。
- `plugin-host` 负责 provider contribution 的注册与快照归集。
- `host-session` 负责为当前 turn 选择 provider，并把最终执行面注入 `agent-runtime`。
- 当前 `server/bootstrap/providers.rs` 中的 OpenAI-only 选择逻辑是过渡实现，最终应从组合根移出，改成 provider registry + contribution 模型。

### 借鉴 `pi-mono` 的 session-as-agent 模式

这次不是要把 Astrcode 做成“看起来像 `pi`”，而是要借它已经验证过的 owner 切分：

- `pi` 的 `Agent` 只负责执行，最多感知 `sessionId`、上下文快照、tool hooks 和 prompt/continue。
- `pi` 的 `AgentSession` 才持有 `SessionManager`、`ResourceLoader`、扩展动作和 session tree/navigation。
- 扩展通过 `sendUserMessage`、`appendEntry`、`setSessionName` 之类的 session action 进入系统，而不是直接篡改 session 真相。

映射到 Astrcode：

- `agent-runtime` 对应 `pi` 的 `Agent`
  - 只执行某个 session/turn
  - 不拥有 session tree、resource discovery、协作 durable truth
- `host-session` 对应 `pi` 的 `AgentSession`
  - 持有 session durable truth
  - 持有 child session / sub-run 协作真相
  - 暴露正式 host-side actions 给 plugin tools/commands 或协议层调用
- `plugin-host` 对应 `pi` 的 extension/resource host
  - 提供 `spawn_agent`、`send_to_child`、`send_to_parent`、`observe_subtree`、`terminate_subtree` 这类协作 surface
  - 但不拥有 collaboration durable truth

和 `pi` 不同的是：Astrcode 的多 agent 协作不是临时 UI 行为，而是正式 durable truth。因此我们借鉴它的“session owner”思路，但不照搬它“没有多 agent 真相层”的限制。

## 关键决策

### 决策 1

- 决策：新建 `agent-runtime`、`host-session`、`plugin-host` 三个 crate，再迁移旧实现；旧 `session-runtime` 最终删除。
- 原因：当前 `session-runtime` 的问题不是实现细节，而是 owner 混合。继续原地瘦身会长期残留“runtime 像 host、host 又像 runtime”的边界污染。
- 备选方案：保留 `session-runtime` crate，只做内部模块重构。
- 为什么没选：crate 名义与真实职责会继续失真，且会诱导后续功能继续往单 crate 堆积。

### 决策 2

- 决策：完全删除 `application` crate，不保留 façade。
- 原因：它没有形成真正稳定的业务层边界，反而把 `session-runtime` 和 `kernel` 的内部事实重新导出到上层。继续保留只会制造多一层穿透与映射成本。
- 备选方案：保留 `application` 作为过渡 use-case façade。
- 为什么没选：这是典型兼容层，会让 server 继续依赖旧心智模型，延缓真正的边界切换。

### 决策 3

- 决策：删除 `kernel` crate，把它拆回真正的 owner 边界。
- 原因：provider/tool/resource orchestration 并不是一个独立产品边界。`agent-runtime` 直接消费 provider/tool/hook 执行面，`host-session` 负责装配，`core` 保留纯合同即可。
- 备选方案：保留更薄的 `kernel` 作为统一门面。
- 为什么没选：会继续引入一层没有独立业务真相的中间层，让依赖方向和调试路径更绕。

### 决策 4

- 决策：统一 plugin descriptor 覆盖 `tools`、`hooks`、`providers`、`resources`、`commands`、`themes`、`prompts`、`skills` 的完整贡献面。
- 原因：只统一 tools/hooks/providers/resources 会继续保留 prompts、skills、themes、commands 的“旁路发现系统”，最终还是多套事实源。
- 备选方案：先只统一运行时贡献面，资源类扩展以后再并入。
- 为什么没选：这会把当前分裂的发现逻辑永久化，后续再并入的成本更高。

### 决策 5

- 决策：`core` 不再作为 DTO 总仓库，owner 专属 DTO 一律迁回 owner crate。
- 原因：当前 `core` 的问题不是“DTO 多”，而是把 session 恢复、projection、workflow、mode、plugin registry、配置存储 ports 这些 owner 私有模型都升格成了跨 crate 依赖。
- 备选方案：继续把新 DTO 放进 `core`，只做文件整理。
- 为什么没选：这只会让 `core` 继续膨胀，延续今天 `ports.rs`、`projection`、`workflow`、`mode` 这种“半核心半实现”的混杂状态。

### 决策 6

- 决策：hooks 平台成为唯一扩展总线，governance、workflow overlay、tool policy、resource discovery、model selection 全部走正式 hooks catalog。
- 原因：Astrcode 已经在 prompt hooks、tool hooks、policy hooks 上积累了多条平行路线，继续增加特判只会让行为来源更隐式。
- 备选方案：保留 governance/workflow 的特判链路，只让部分能力走 hooks。
- 为什么没选：这会让 hooks 失去“统一总线”的意义，只变成又一套附属扩展点。

### 决策 7

- 决策：builtin plugin 与 external plugin 共享统一 descriptor、registry、snapshot 和 hook surface，但采用不同执行后端。
- 原因：统一事实面是必须的，但热路径性能和进程隔离需求不能强行合并成同一性能模型。
- 备选方案：全部外部进程化，或 builtin / external 完全两套实现。
- 为什么没选：前者会把热路径延迟和失败面放大，后者会重新回到双轨事实源。

### 决策 8

- 决策：新 turn 固定绑定启动时的 `PluginActiveSnapshot`，reload 只影响后续 turn。
- 原因：这样才能保证执行一致性，避免中途切换 snapshot 导致同一 turn 的工具、hook、provider、prompt 语义漂移。
- 备选方案：reload 后立即让所有在途 turn 读取新 snapshot。
- 为什么没选：会造成执行不确定性和调试困难，尤其在流式 provider / tool execution 期间。

### 决策 9

- 决策：多 agent 协作继续遵循“一个 session 即一个 agent”的原则，所有父子 session 关系、`SubRunHandle`、input queue、结果投递、跨 turn 取消与 lineage durable truth 一律归 `host-session`；`agent-runtime` 只保留最小 child-session 执行合同。
- 原因：Astrcode 现有 sub-run 协作已经明确依赖事件日志、query/read model、parent/child lineage 与 turn 终态，这些都属于宿主 durable truth，而不是 live runtime loop。
- 备选方案：单独新建 collaboration crate，或继续把协作模型分散留在 `core + application + session-runtime`。
- 为什么没选：前者会把本质上依赖 session durable truth 的能力又切出一个没有独立真相的中间层；后者会延续今天最严重的跨 owner 污染。

### 决策 10

- 决策：协作能力对外通过 plugin/tool/command surface 暴露，但这些 surface 只负责把动作提交给 `host-session`，不拥有 collaboration durable truth。
- 原因：这样既能满足“其他一切通过 plugin 提供”的 product surface 目标，也能保持 session / sub-run / input queue / result delivery 的唯一真相仍在 `host-session`。
- 备选方案：让 `plugin-host` 或扩展 handler 直接维护 child session 状态，或让 runtime 内部直接暴露协作特判入口。
- 为什么没选：前者会让扩展层越权持有 durable truth，后者会重新把协作逻辑塞回 runtime 主链。

## 数据流 / 控制流 / 错误流

### 启动与装配

1. `server` 启动时装配 storage、provider adapters、tool adapters、prompt/resource adapters。
2. `plugin-host` 发现 builtin plugin 定义和 external plugin 来源。
3. `plugin-host` 将所有来源解析为 `PluginDescriptor`，校验字段、冲突、权限、执行后端可用性。
4. 校验通过后构建 `PluginActiveSnapshot` 并提交为 active revision。
5. `host-session` 使用 active snapshot 生成资源目录、模型目录和 host 可用能力视图。

这里的关键点是：`server` 只负责把“有哪些后端”交给 `plugin-host`，真正的合并、排序、冲突解析、激活都由 `plugin-host` owner 负责。

### 正常 turn 执行流

1. 外部输入先进入 `host-session`。
2. `host-session` 触发 `input` hook，允许短路、转换或已处理。
3. `host-session` 根据当前 `HostSessionSnapshot`、模型选择结果、`PluginActiveSnapshot` 组装 `AgentRuntimeExecutionSurface`。
4. `agent-runtime` 开始 turn：
   - 触发 `turn_start`
   - 运行 `context`
   - 运行 `before_agent_start`
   - 运行 `before_provider_request`
   - 发起 provider 流式请求
5. 如果模型返回工具调用：
   - `agent-runtime` 触发 `tool_call`
   - 执行工具
   - 触发 `tool_result`
   - 决定是否继续下一轮 provider 请求
6. turn 结束时触发 `turn_end`。
7. `agent-runtime` 只返回执行结果和运行时事件；durable 写入由 `host-session` 完成。

### compaction / branch / fork / model 切换流

- compaction 由 `host-session` 决策和执行，执行前触发 `session_before_compact`。
- branch / fork 是 `host-session` 的 durable 操作，不经过 `agent-runtime`。
- 模型切换由 `host-session` 发起，经过 `model_select` hook 校验、重写或拒绝，再更新后续 turn 的执行面。

### 多 agent 协作流

1. 父 session 的某个 turn 在 `host-session` 内决定发起子 agent。
2. `host-session` 创建新的 child session，并把它视为新的 agent 实例，而不是在同一 session 内切换“子人格”。
3. `host-session` 追加 sub-run started / lineage / input queue 相关事件，生成 `SubRunHandle` 并更新 `InputQueueProjection`。
4. `host-session` 为 child session 组装新的 `AgentRuntimeExecutionSurface`，然后调用 `agent-runtime` 执行 child turn。
5. child turn 完成后，`agent-runtime` 只返回最小执行结果；sub-run finished、结果投递、父 turn 唤醒、跨 turn cancel 清理由 `host-session` 落 durable truth。
6. 如果父 turn 被取消或中断，取消传播先由 `host-session` 决定并记录，再把 cancel token 传递给对应 child runtime。

这个流程的关键不是“能不能启动子 agent”，而是“子 agent 是否仍然是一个完整可恢复、可查询、可分叉的 session”。因此 collaboration 真相必须留在 `host-session`。

### 协作能力暴露流

1. `plugin-host` 在 active snapshot 中暴露 collaboration 相关 tools/commands，例如 `spawn_agent`、`send_to_child`、`send_to_parent`、`observe_subtree`、`terminate_subtree`。
2. LLM、CLI、RPC 或其他宿主通过这些统一 surface 发起协作动作。
3. surface handler 不直接改 session durable truth，而是调用 `host-session` 的正式 use-case surface。
4. `host-session` 负责 child session 创建、sub-run 事件落库、input queue 投递、结果回传与取消传播。
5. 如需执行 child turn，再由 `host-session` 调用 `agent-runtime`。

这样可以同时满足两件事：

- 对外看，协作能力和其他 builtin/external 扩展一样，走统一 plugin surface。
- 对内看，协作状态仍只有一个 owner，不会被扩展层复制一份真相。

### resource discovery 流

1. `plugin-host` 在 active snapshot 构建后或 reload 后触发 `resources_discover`。
2. 各 plugin 贡献 `skills`、`prompts`、`themes`、`commands`、其他资源入口。
3. `plugin-host` 聚合为统一资源目录，供 `host-session`、CLI、server 路由或 UI 消费。

### 错误流

#### plugin 加载 / reload

- `plugin-host` 先构建 candidate snapshot，再做原子提交。
- 任一 descriptor 校验失败、backend 启动失败、冲突无法消解时，candidate 作废，旧 active snapshot 保持不变。
- reload 错误只影响新 revision，不污染当前 active turn。

#### hook 执行

- 每个 hook 按 `failure_policy` 处理：
  - `fail_closed`：阻断当前流程。
  - `fail_open`：记录报告后继续。
  - `report_only`：只产出观测结果，不改变主流程。
- hook 只能返回受约束 effect，不能直接写 durable truth。

#### provider / tool 执行

- `agent-runtime` 负责把 provider/tool 错误转成统一运行时结果。
- 会话日志、read model、终态快照的持久化仍由 `host-session` owner 处理。
- 在取消、中断、部分流输出场景下，`agent-runtime` 产出“不完整但一致”的执行结果，`host-session` 决定如何写入事件日志与恢复点。

## 与 DTO / Spec 的对应关系

### 对 `agent-runtime-core` 的落实

- `AgentRuntimeExecutionSurface` 是 `host-session -> agent-runtime` 的唯一正式输入，但它归属 `agent-runtime` crate，而不是 `core`。
- `agent-runtime` 只消费纯数据输入，不自行做资源发现或持久化。
- `HookEventEnvelope`、`HookEffect`、`HookExecutionReport` 覆盖 runtime 事件触发和 effect 解释，但它们属于 hooks/runtime owner，而不是默认进入 `core`。
- 即使存在 child-session 启动场景，`agent-runtime` 也只负责执行某个 session/turn，不负责维护 `SubRunHandle`、input queue、结果投递或 parent/child lineage durable truth。

### 对 `host-session-runtime` 的落实

- `HostSessionSnapshot` 是 host 层对 durable truth / read model 的统一视图，归属 `host-session`。
- 输入入口、compaction、branch/fork、query/read model、turn 组装全部归 `host-session` owner。
- `session_before_compact`、`model_select`、`input` 等事件由 `host-session` 触发。
- 多 agent 协作里的 `SubRunHandle`、`InputQueueProjection`、parent/child lineage、subrun finished/cancel 事件、结果投递与父 turn 唤醒也归 `host-session` owner。

### 对 `plugin-host-runtime` 的落实

- `PluginDescriptor` 是统一输入模型，归属 `plugin-host`。
- `PluginActiveSnapshot` 是唯一生效快照，归属 `plugin-host`。
- builtin / external plugin 只在 backend 执行方式不同，不在描述模型和装配流程上分叉。

### 对 `lifecycle-hooks-platform` 的落实

- hooks catalog、dispatch mode、failure policy、effect 限制都以 `HookDescriptor` + `HookEventEnvelope` + `HookEffect` 表达。
- governance prompt augment 通过 `augment_prompt -> PromptDeclaration` 映射进入既有 prompt 管线。

### 对 DTO 的补充决策

- `ProviderDescriptor`、`ResourceDescriptor`、`CommandDescriptor`、`ThemeDescriptor`、`PromptDescriptor`、`SkillDescriptor` 都属于 `plugin-host` 的子贡献面。
- `SessionRecoveryCheckpoint`、`RecoveredSessionState`、各类 projection/read model 不再属于 `core`，而是 `host-session` 自有模型。
- `LlmRequest`、`LlmOutput`、provider stream event、tool/runtime execution context 不再通过 `core::ports` 这个 mega 模块承载，而是迁到各自 owner 合同模块。
- `workflow`、`plugin::registry`、`projection`、`session_catalog` 不再被视为 core 语义，而是迁出或删除；`mode` 先拆成 plugin-host owner DSL 与 core 共享 wire/control 合同。

这些调整的目标不是“减少 struct 数量”，而是让 DTO 和 owner 一一对应，避免 `core` 继续成为跨边界耦合中心。

## 风险与取舍

- [大范围 crate 重构] -> 先更新 `PROJECT_ARCHITECTURE.md`，再按 owner 拆 crate；每一步都以编译边界和 bootstrap 切换为验收点。
- [删除 `application` / `kernel` 影响面大] -> 不做长期兼容层，但允许在迁移阶段保留短生命周期的内部桥接模块，桥接模块不得对外暴露为正式 API。
- [收缩 `core` 时类型搬迁范围大] -> 先定义“哪些模型真的是跨 owner 共享语义”，其余按 owner 逐批迁移；迁移顺序以删除 `core::ports`、`core::projection`、`core::plugin`、`core::workflow/mode` 为主线。
- [多 agent 协作跨 crate 现状复杂] -> 先用“一个 session 即一个 agent”固定语义，再把 `SubRunHandle`、input queue、cancel/result-delivery 真相统一迁入 `host-session`，避免 runtime 和 host 各记一套状态。
- [plugin 贡献面扩大后校验复杂度上升] -> 所有贡献先落到 `PluginDescriptor`，统一做 schema 校验、命名冲突校验、优先级排序和 backend 就绪检查。
- [hooks 事件增多后行为更隐式] -> 强制 event catalog、owner、dispatch mode、failure policy 全部显式化，并输出 `HookExecutionReport`。
- [builtin 与 external backend 差异] -> 保持统一 descriptor / snapshot，不强行统一性能模型；host 只承诺行为语义一致。
- [reload 与在途 turn 一致性] -> turn 固定绑定 snapshot revision，新 revision 只作用于后续 turn。

## 实施与迁移

### 第 0 步：更新架构权威文档

- 先修改 `PROJECT_ARCHITECTURE.md`。
- 删除 `application`、`kernel`、monolith `session-runtime` 的长期权威定义。
- 写入 `agent-runtime`、`host-session`、`plugin-host` 的新边界、依赖方向和 owner 约束。

### 第 1 步：建立新 crate 骨架

- 新建：
  - `crates/agent-runtime`
  - `crates/host-session`
  - `crates/plugin-host`
- 在 `core` 中只保留新的共享语义和值对象，不把 owner 专属 DTO 再放回去。

### 第 2 步：迁移最小 runtime 核心

- 从旧 `session-runtime` 中迁出 turn loop、provider 调用、tool dispatch、hook dispatch、流式状态机到 `agent-runtime`。
- 明确 `agent-runtime` 不再依赖 session catalog、query、projection、branch/fork。

### 第 3 步：迁移 host 会话能力

- 从旧 `session-runtime` 中迁出事件日志、恢复、投影、query、branch/fork、compact、session catalog 到 `host-session`。
- 从 `core/application/session-runtime` 中迁出 `SubRunHandle`、`InputQueueProjection`、协作 executor 合同、subrun finished/cancel 持久化、child session 启动与结果投递逻辑到 `host-session`。
- 迁移期使用 owner bridge：执行/read-model 类型先通过 `host-session` 对外暴露；`ChildAgentRef`、`ChildSessionNode`、`ChildSessionLineageKind` 暂留 `core`，因为它们嵌入 `ChildSessionNotification` 和 durable event payload。
- 由 `host-session` 负责组装 `AgentRuntimeExecutionSurface` 并调用 `agent-runtime`。

### 第 3.5 步：收缩 core

- 拆掉 `crates/core/src/ports.rs` 这一类 mega 合同文件，按 owner 迁入 `agent-runtime`、`host-session`、`plugin-host` 或 `support`。
- 迁出或删除 `crates/core/src/projection`、`workflow.rs`、`plugin/registry.rs`、`session_catalog.rs`、owner 专属 observability / config store 模型；`mode` 采取窄桥接策略，避免 `core` 反向依赖 plugin-host。
- 保留 `ids`、消息模型、`CapabilitySpec`、极少数共享 prompt/hook 语义和值对象。

### 第 4 步：迁移 plugin 宿主能力

- 把当前 `crates/plugin` 的 loader / process / peer / supervisor / invoker / worker 协议收敛到 `plugin-host`。
- 新增 active snapshot、descriptor 校验、reload candidate/commit/rollback、resource discovery。
- 将 builtin tools、governance、workflow overlay、MCP bridge 逐步改造成 builtin plugin 贡献。

### 第 5 步：切换组合根

- 先重写 `crates/server/src/bootstrap/runtime.rs` 的 plugin/provider/resource 生效事实来源，进入短生命周期 bridge：
  - `server` 仍保留现有 HTTP/API 调用所需的 `application` / `kernel` / `session-runtime` 外壳。
  - builtin tools、MCP tools、collaboration tools、provider 与 external plugin descriptor 必须合并为同一组 `PluginDescriptor[]`。
  - 这组 descriptor 通过单个 reload bridge 产出 `PluginActiveSnapshot`、`ResourceCatalog`、`ProviderContributionCatalog`，server 后续 provider、prompt facts、resource catalog 只消费该产物。
- governance / mode / workflow 也按 bridge 处理：
  - builtin modes 与 plugin-declared modes 先进入 `PluginDescriptor.modes`，并随 `PluginActiveSnapshot` 一起提交。
  - server 从 snapshot 中的 mode 贡献构建 `ModeCatalog`，旧 `GovernanceSurfaceAssembler` / `AppGovernance` 只消费这个 catalog。
  - `WorkflowOrchestrator` 的最终 hooks 化和旧 owner 删除留到旧 API 调用方切换后执行，不在 bridge 阶段同时跨越。
- 旧 `application`、`kernel`、旧 `session-runtime` 的正式依赖删除放到第 6 步执行；只有当协议/API 调用方已经切到 `host-session + agent-runtime + plugin-host` 后，才删除这些旧边界。

### 第 6 步：分阶段删除旧边界

第 6 步必须先迁移调用方，再删除 crate。当前 bridge 已经把 plugin/provider/resource/mode 的生效事实收敛到 `plugin-host`，但 server HTTP/API 面仍编译依赖旧 `application` / `kernel` / `session-runtime` / `plugin`。因此删除顺序固定为：

1. 切换 server 运行时 API 面，按调用面分批推进：
   - config / model：从 `App::config()` 迁到 server-owned config/profile service 或新 owner service。
   - session catalog CRUD / fork / catalog stream：list/create/delete/delete_project/fork/catalog stream 先迁到 server-owned `host-session::SessionCatalog`，fork 可短期保留 server-side plan artifact copy bridge。
   - turn mutation 分阶段迁移：
     1. 先在 `host-session` 建立 submit/compact/interrupt owner 合同，明确 durable turn mutation 与 governance/workflow/skill-invocation bridge 的边界。
     2. 再把 submit acceptance、turn lease、branch-on-busy 目标解析迁到 `host-session::SessionCatalog`，让 `application` 不再决定 submit target。
     3. 再接通 `agent-runtime::RuntimeTurnEvent` 到 `host-session` 事件持久化、投影、broadcast、checkpoint 路径。
     4. 再迁移 compact / interrupt 的 owner 行为，包括 manual compact 延迟登记、cancel token、terminal cancelled event 与 pending compact flush。
     5. 最后切换 server submit/compact/interrupt 路由，不再调用 `application::App` turn/session mutation use-case。
     迁移期间 governance/workflow/skill-invocation 可保留短生命周期 bridge，但该 bridge 不得拥有 durable turn mutation truth。
   - session mode：list_modes/get_session_mode/switch_mode 迁到 `plugin-host` mode catalog 与 `host-session` mode state owner。
   - conversation / terminal read-model：terminal facts、conversation stream、authoritative summary 迁到 `host-session` query/read-model 与 server projection adapter。
   - composer / resource discovery：composer options、skills、commands、prompts、themes 迁到 `plugin-host::ResourceCatalog` / descriptor-derived catalog。
   - agent collaboration：agent status、root execute、close/observe 和 collaboration tools 迁到 `host-session` collaboration use-case 与 `plugin-host` surface。
   - 最后移除 `ServerRuntime.app` / `AppState.app`；协议映射可以留在 server thin adapter 中，但 `application::App` 不再是业务入口。
2. 切换旧 `kernel` 能力面：`CapabilityRouter` / `KernelGateway` / `SurfaceManager` 的调用方改为消费 `plugin-host` active snapshot、tool dispatch、provider/resource catalog 或 `agent-runtime` 执行面。
3. 切换旧 `session-runtime` 剩余调用面：catalog、query/read-model、observe、branch/fork、compaction、turn 提交和 child-session 驱动全部归 `host-session + agent-runtime`。
4. 切换旧 `plugin` 进程宿主边界：loader / supervisor / process / peer / worker protocol 作为 `plugin-host` external backend 生效，不再暴露旧 `astrcode-plugin` crate。
5. 删除 workspace 中的旧 crate 与依赖规则：`crates/application`、`crates/kernel`、旧 `crates/session-runtime`、旧 `crates/plugin` 不再作为正式 crate 参与编译。
6. 执行 `0.*` 删除验收：清理旧 port、旧 re-export、旧 helper、旧 bootstrap 特判和 owner-only core 暴露，确认仓库中无残留正式依赖路径。

### 回滚策略

- 不提供长期双轨回滚。
- 在组合根正式切换前，每个迁移阶段都可以通过 git revert 回退。
- 一旦 `server` 切换到新边界，旧路径应直接删除，回滚只能基于版本回退，不保留运行时开关。

## 验证方案

- 架构验证：
  - 更新并执行 `node scripts/check-crate-boundaries.mjs`
  - 确认 `server` 只做组合根，新依赖方向满足文档约束
- 编译验证：
  - `cargo check --workspace`
- 行为验证：
  - 为 `agent-runtime` 编写 turn 执行、取消、tool_call/tool_result、provider streaming 测试
  - 为 `host-session` 编写事件日志恢复、branch/fork、compaction、model_select 测试
  - 为 `plugin-host` 编写 descriptor 校验、snapshot commit/rollback、reload、一致性测试
  - 为 hooks 平台编写 event owner、dispatch mode、failure policy、effect 解释测试
- 集成验证：
  - 验证 builtin plugin 与 external plugin 在同一 active snapshot 中可共同提供 tools/hooks/providers/resources
  - 验证 reload 失败不会污染旧 snapshot
  - 验证 in-flight turn 固定绑定旧 snapshot，新 turn 使用新 snapshot

## 未决问题

无。
