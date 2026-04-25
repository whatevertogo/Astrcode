## 概览

本次 change 的数据模型分成四组，但需要先强调一个收缩原则：

- 不是所有 DTO 都应该继续放进 `core`
- DTO 的归属优先跟 owner，而不是跟“它看起来像纯数据”
- `core` 只保留真正跨 owner 共享且长期稳定的值对象
- `agent-runtime`、`host-session`、`plugin-host` 各自拥有自己的内部/公共合同

在这个原则下，本次 change 的数据模型分成四组：

- 新增核心模型：
  - `PluginDescriptor`
  - `PluginActiveSnapshot`
  - `HookMatcher`
  - `HookDescriptor`
  - `HookEventEnvelope`
  - `HookEffect`
  - `HookExecutionReport`
  - `AgentRuntimeExecutionSurface`
  - `HostSessionSnapshot`
- 迁移并继续复用的协作模型：
  - `SubRunHandle`
  - `InputQueueProjection`
- 修改/替代模型：
  - 旧 `PluginManifest` 不再足够表达完整贡献面，需要被新的 `PluginDescriptor` 取代
  - 旧 `core::HookInput` / `HookOutcome` 只覆盖窄版 tool/compact hook，需要被更宽的 hooks 事件与 effect 模型替代
  - 旧 monolith `session-runtime` 对外 surface 会被拆成 `agent-runtime` 与 `host-session` 两类模型
  - 旧 `core + application + session-runtime` 三处分散的 subagent/subrun 协作模型，会收敛成 `host-session` durable truth + `agent-runtime` 最小执行合同
- 直接复用的现有模型：
  - `CapabilitySpec`
  - `PromptDeclaration`
  - `PromptGovernanceContext`
  - `SessionObserveSnapshot`
  - `TurnTerminalSnapshot`
- 建模目标：
  - 让 `agent-runtime` 只消费纯数据的执行面
  - 让 `plugin-host` 统一管理 builtin / external plugin 的贡献
  - 让 hooks 的事件、effect、执行报告都可以独立序列化、校验和记录
  - 让 DTO 跟随 owner 收缩，不再继续把 owner 专属模型堆入 `core`

## 归属原则

### `core` 保留什么

- `ids`
- LLM / tool / message 相关的基础消息模型
- `CapabilitySpec`
- 极少数跨 owner 共享的 prompt 声明和值对象
- hooks 的稳定事件键、effect kind 这类共享语义枚举

### `core` 不再保留什么

- session recovery checkpoint、projection snapshot、read model
- workflow / mode / session catalog
- plugin manifest / plugin registry / active snapshot
- provider / prompt / resource / config 的 owner 专属 ports
- observability report、runtime execution report 这类 owner 局部模型

### owner 模型归属

- `agent-runtime`
  - `AgentRuntimeExecutionSurface`
  - runtime hook payload / effect interpreter input
  - provider/tool 执行上下文与结果
- `host-session`
  - `HostSessionSnapshot`
  - `SessionRecoveryCheckpoint`
  - `RecoveredSessionState`
  - projection / query / observe 结果
  - `SubRunHandle`
  - `InputQueueProjection`
  - 协作 executor 合同、结果投递与协作终态快照
  - parent/child lineage 的 owner 仍是 host-session；迁移期 `ChildAgentRef`、`ChildSessionNode`、`ChildSessionLineageKind` 作为 durable event DTO 组成部分暂留 `core`
- `plugin-host`
  - `PluginDescriptor`
  - `PluginActiveSnapshot`
  - `HookDescriptor`
  - `ProviderDescriptor`
  - `ResourceDescriptor`
  - `CommandDescriptor`
  - `ThemeDescriptor`
  - `PromptDescriptor`
  - `SkillDescriptor`

## 模型清单

### `HookMatcher`

- 用途：描述某个 hook 在命中指定事件后，还需要如何进一步收窄匹配范围
- 类型：值对象
- 所属边界：hooks platform / `plugin-host`
- 来源：`HookDescriptor`
- 去向：dispatcher 的匹配阶段

#### 表达形式

- `All`
  - 匹配当前事件下的全部实例，是第一阶段默认值
- `ToolNames(Vec<String>)`
  - 仅匹配指定工具名
- `AgentIds(Vec<AgentId>)`
  - 仅匹配指定 agent
- `SessionIds(Vec<SessionId>)`
  - 仅匹配指定 session

#### 校验规则与不变量

- `HookMatcher` 只能收窄当前 `event` 的命中范围，不能声明新的事件类型
- 第一阶段没有更细的上下文条件时，默认使用 `All`
- `ToolNames` 只对 `tool_call` / `tool_result` 这类工具相关事件生效

### `PluginDescriptor`

- 用途：描述一个 plugin 的完整贡献面，是 `plugin-host` 的正式输入模型
- 类型：DTO
- 所属边界：`plugin-host` 对内/对外统一装配边界
- 来源：builtin plugin 定义、external plugin manifest/handshake、远程 plugin 元数据
- 去向：`plugin-host` registry、reload candidate snapshot、资源发现、hooks 注册、tool/provider 注册

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `plugin_id` | `string` | 是 | - | 全局唯一、kebab-case | 稳定插件标识 |
| `display_name` | `string` | 是 | - | 非空 | 展示名称 |
| `version` | `string` | 是 | - | 语义版本或稳定 revision | 插件版本/修订 |
| `source_kind` | `"builtin" \| "process" \| "command" \| "http"` | 是 | - | 枚举 | 执行来源 |
| `source_ref` | `string` | 是 | - | 非空 | 可执行路径、URL、内联实现名等 |
| `enabled` | `boolean` | 是 | `true` | - | 是否参与当前候选装配 |
| `priority` | `i32` | 否 | `0` | 数值越大优先级越高 | 冲突解析顺序 |
| `tools` | `CapabilitySpec[]` | 否 | `[]` | 名称唯一 | LLM 可调用工具贡献 |
| `hooks` | `HookDescriptor[]` | 否 | `[]` | `hook_id` 唯一 | 生命周期 hooks 贡献 |
| `providers` | `ProviderDescriptor[]` | 否 | `[]` | provider id 唯一 | 模型/provider 贡献 |
| `resources` | `ResourceDescriptor[]` | 否 | `[]` | 可按 kind 聚合 | 原始资源贡献 |
| `commands` | `CommandDescriptor[]` | 否 | `[]` | command id 唯一 | slash/命令贡献 |
| `themes` | `ThemeDescriptor[]` | 否 | `[]` | theme id 唯一 | 主题贡献 |
| `prompts` | `PromptDescriptor[]` | 否 | `[]` | prompt id 唯一 | prompt 模板贡献 |
| `skills` | `SkillDescriptor[]` | 否 | `[]` | skill id 唯一 | skill 贡献 |

#### 与其他模型的关系

- 与 `PluginActiveSnapshot`：`PluginDescriptor` 是 snapshot 构建输入
- 与 `HookDescriptor`：一个 plugin 可以贡献 0..N 个 hooks
- 与 `AgentRuntimeExecutionSurface`：部分字段会被降维投影到执行面

#### 校验规则与不变量

- 同一 `plugin_id` 在同一个 active snapshot 中只能出现一次
- 同一 plugin 内部的 `tools/hooks/providers/resources/commands/themes/prompts/skills` 标识不得冲突
- `source_kind = "builtin"` 时，`source_ref` 必须指向已注册的进程内实现
- `source_kind != "builtin"` 时，必须提供可解析的执行入口

#### 生命周期 / 状态变化

- `discovered -> validated -> candidate -> active -> stale`

#### 映射关系

- `plugin manifest / handshake -> PluginDescriptor -> PluginActiveSnapshot`

### `PluginActiveSnapshot`

- 用途：表示某一时刻真正生效的统一 plugin 贡献面
- 类型：视图模型
- 所属边界：`plugin-host -> host-session / agent-runtime`
- 来源：由多个 `PluginDescriptor` 经过校验、冲突解析和合并得到
- 去向：`agent-runtime`、`host-session`、统一 discovery、reload rollback

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `snapshot_id` | `string` | 是 | - | 全局唯一 | 当前生效面版本 |
| `revision` | `u64` | 是 | - | 单调递增 | reload 版本号 |
| `plugin_ids` | `string[]` | 是 | - | 去重 | 参与本次快照的 plugin 列表 |
| `tools` | `CapabilitySpec[]` | 是 | `[]` | 名称唯一 | 生效工具集合 |
| `hooks` | `HookDescriptor[]` | 是 | `[]` | `hook_id` 唯一 | 生效 hooks 集合 |
| `providers` | `ProviderDescriptor[]` | 是 | `[]` | provider id 唯一 | 生效 provider 集合 |
| `resources` | `ResourceDescriptor[]` | 是 | `[]` | 可按 kind 聚合 | 生效资源集合 |
| `commands` | `CommandDescriptor[]` | 是 | `[]` | command id 唯一 | 生效命令集合 |
| `themes` | `ThemeDescriptor[]` | 是 | `[]` | theme id 唯一 | 生效主题集合 |
| `prompts` | `PromptDescriptor[]` | 是 | `[]` | prompt id 唯一 | 生效 prompt 集合 |
| `skills` | `SkillDescriptor[]` | 是 | `[]` | skill id 唯一 | 生效 skills 集合 |

#### 与其他模型的关系

- 与 `PluginDescriptor`：由多个 descriptor 聚合而来
- 与 `AgentRuntimeExecutionSurface`：执行面只消费其中相关子集

#### 校验规则与不变量

- 任何 active snapshot 必须是完整可用的整体，不能是半成功状态
- reload 失败时不得产生新 active snapshot
- 进行中的 turn 绑定旧 snapshot，新 turn 绑定新 snapshot

#### 生命周期 / 状态变化

- `candidate -> active -> superseded -> garbage_collectable`

#### 映射关系

- `PluginDescriptor[] -> PluginActiveSnapshot -> AgentRuntimeExecutionSurface`

### `HookDescriptor`

- 用途：描述一个 hook 的注册信息、触发事件、匹配条件与执行策略
- 类型：DTO
- 所属边界：`plugin-host -> hooks platform`
- 来源：builtin hook 定义、external plugin hook 声明
- 去向：hooks registry、active snapshot、dispatch pipeline

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `hook_id` | `string` | 是 | - | 全局唯一 | 稳定 hook 标识 |
| `plugin_id` | `string` | 是 | - | 必须指向已存在 plugin | 所属插件 |
| `event` | `HookEventKey` | 是 | - | 枚举值 | 订阅的事件名 |
| `stage` | `"runtime" \| "host" \| "resource"` | 是 | - | 枚举 | 所属执行阶段 |
| `dispatch_mode` | `"sequential" \| "pipeline" \| "cancellable" \| "intercept" \| "modify" \| "short_circuit"` | 是 | - | 与 `event` 必须匹配 | 分发语义 |
| `priority` | `i32` | 否 | `0` | 数值越大越先执行 | 稳定顺序 |
| `matcher` | `HookMatcher` | 否 | `All` | 只能收窄命中范围 | 细粒度匹配 |
| `timeout_ms` | `u64` | 否 | `5000` | `> 0` | 单次执行超时 |
| `failure_policy` | `"fail_closed" \| "fail_open" \| "report_only"` | 是 | - | 枚举 | 失败语义 |
| `handler_ref` | `string` | 是 | - | 非空 | 指向内联实现、命令、进程或远程入口 |
| `enabled` | `boolean` | 是 | `true` | - | 是否生效 |

#### 与其他模型的关系

- 与 `HookEventEnvelope`：按 `event` 匹配后执行
- 与 `HookEffect`：执行后产出 0..N 个 effect
- 与 `HookExecutionReport`：每次执行都会记录一条报告

#### 校验规则与不变量

- `event` 与 `dispatch_mode` 必须匹配对应的正式事件语义
- `hook_id` 在同一个 active snapshot 中必须唯一
- `matcher` 默认值为 `All`
- `matcher` 只能收窄命中范围，不能声明新的事件类型

#### 生命周期 / 状态变化

- `declared -> validated -> registered -> active -> stale`

#### 映射关系

- `plugin hook declaration -> HookDescriptor -> HookEventEnvelope -> HookEffect[]`

### `HookEventEnvelope`

- 用途：统一描述一次 hook 触发时的输入上下文
- 类型：事件
- 所属边界：`hooks platform` 运行时事件模型；实现归属具体 owner crate，而不是 `core`
- 来源：运行时、host、resource discovery
- 去向：hooks dispatcher、hooks report、诊断日志

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `event_id` | `string` | 是 | - | 全局唯一 | 本次触发实例 id |
| `event` | `HookEventKey` | 是 | - | 枚举值 | 事件名 |
| `session_id` | `string` | 否 | `null` | 与 session 相关时必填 | 所属 session |
| `turn_id` | `string` | 否 | `null` | 与 turn 相关时必填 | 所属 turn |
| `agent_id` | `string` | 否 | `null` | 与 agent 相关时必填 | 所属 agent |
| `source_owner` | `"agent-runtime" \| "host-session" \| "plugin-host"` | 是 | - | 枚举 | 触发 owner |
| `timestamp_ms` | `u64` | 是 | - | 单调时间戳 | 触发时间 |
| `payload` | `serde_json::Value` | 是 | - | 必须符合该 `event` 的 schema | 事件载荷 |
| `snapshot_id` | `string` | 是 | - | 非空 | 绑定的 active snapshot |

#### 与其他模型的关系

- 与 `HookDescriptor`：按 `event` 和 `matcher` 选出命中的 hooks
- 与 `HookExecutionReport`：是报告的主键引用之一

#### 校验规则与不变量

- `payload` 必须与 `event` 对应的 schema 一致
- 同一 `event_id` 不得重复执行两次正式 dispatch
- `source_owner` 必须和正式 event catalog 中定义的 owner 一致

#### 生命周期 / 状态变化

- `created -> dispatched -> settled -> reported`

#### 映射关系

- `runtime/host trigger -> HookEventEnvelope -> dispatcher`

### `HookEffect`

- 用途：表达 hook 对当前流程施加的受约束影响
- 类型：其他
- 所属边界：hooks platform -> owner runtime/host
- 来源：hook handler 执行结果
- 去向：`agent-runtime`、`host-session`、`plugin-host` 的 effect interpreter

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `effect_id` | `string` | 是 | - | 全局唯一 | effect 实例 id |
| `event_id` | `string` | 是 | - | 必须指向触发事件 | 来源事件 |
| `kind` | `"block" \| "cancel_turn" \| "transform_input" \| "augment_prompt" \| "mutate_payload" \| "override_tool_result" \| "resource_path" \| "model_hint" \| "diagnostic"` | 是 | - | 枚举 | effect 类型 |
| `target` | `string` | 是 | - | 非空 | 作用对象，如 tool/model/prompt/resources |
| `payload` | `serde_json::Value` | 是 | - | 必须符合 `kind` schema | effect 数据 |
| `terminal` | `boolean` | 是 | `false` | - | 是否短路后续处理 |
| `diagnostic_message` | `string` | 否 | `null` | 可为空 | 面向日志/观测的说明 |

#### 与其他模型的关系

- 与 `HookExecutionReport`：报告中会汇总 effect 列表
- 与 `PromptDeclaration`：`augment_prompt` 会被映射成 `PromptDeclaration`

#### 校验规则与不变量

- effect 只能来自正式允许的 effect 集合
- effect 不得直接写 durable truth
- `kind = "augment_prompt"` 时，必须能映射到既有 `PromptDeclaration` 或 `PromptGovernanceContext`
- `kind = "cancel_turn"` 时，`terminal` 必须为 `true`，且 `payload` 必须包含可对用户或上层宿主解释的终止原因

#### 生命周期 / 状态变化

- `emitted -> interpreted -> applied / rejected`

#### 映射关系

- `HookEventEnvelope + HookDescriptor -> HookEffect[] -> owner interpreter`

### `HookExecutionReport`

- 用途：记录某次 hook 执行的结果，支撑 observability 与调试
- 类型：事件
- 所属边界：hooks platform observability；归属 hooks/runtime owner
- 来源：dispatcher
- 去向：日志、观测面板、调试导出、reload 诊断

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `report_id` | `string` | 是 | - | 全局唯一 | 执行报告 id |
| `event_id` | `string` | 是 | - | 必须存在 | 对应触发事件 |
| `hook_id` | `string` | 是 | - | 必须存在 | 对应 hook |
| `status` | `"matched" \| "skipped" \| "succeeded" \| "blocked" \| "failed_open" \| "failed_closed"` | 是 | - | 枚举 | 执行结果 |
| `duration_ms` | `u64` | 是 | - | `>= 0` | 执行耗时 |
| `effects` | `HookEffect[]` | 是 | `[]` | - | 产出的 effect |
| `error_code` | `string` | 否 | `null` | 可为空 | 错误码 |
| `error_message` | `string` | 否 | `null` | 可为空 | 错误信息 |

#### 与其他模型的关系

- 与 `HookEventEnvelope`：多条 report 可关联同一个 event
- 与 `HookDescriptor`：一条 report 只对应一个 hook

#### 校验规则与不变量

- `status = "succeeded"` 时，`error_*` 应为空
- `status` 为失败态时，必须遵守 `failure_policy`
- 报告属于观测事实，不属于 durable session truth

#### 生命周期 / 状态变化

- `pending -> finalized -> exported`

#### 映射关系

- `dispatcher result -> HookExecutionReport -> observability`

### `AgentRuntimeExecutionSurface`

- 用途：`host-session` 传给 `agent-runtime` 的最小执行面
- 类型：内部核心模型
- 所属边界：`host-session -> agent-runtime`
- 来源：`host-session` 根据当前 session 状态和 `PluginActiveSnapshot` 组装
- 去向：`agent-runtime` turn loop

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `session_id` | `string` | 是 | - | 非空 | 当前会话 |
| `turn_id` | `string` | 是 | - | 非空 | 当前 turn |
| `agent_id` | `string` | 是 | - | 非空 | 当前 agent |
| `model_ref` | `string` | 是 | - | 非空 | 当前模型引用 |
| `provider_ref` | `string` | 是 | - | 非空 | 当前 provider 引用 |
| `tool_specs` | `CapabilitySpec[]` | 是 | `[]` | 名称唯一 | 生效工具集合 |
| `hook_snapshot_id` | `string` | 是 | - | 非空 | 绑定的 hooks snapshot |
| `prompt_declarations` | `PromptDeclaration[]` | 是 | `[]` | 可为空 | 初始 prompt 声明 |
| `prompt_governance` | `PromptGovernanceContext` | 否 | `null` | 可为空 | prompt 治理上下文 |
| `limits` | `ResolvedExecutionLimitsSnapshot` | 否 | `null` | 可为空 | 执行限制 |

#### 与其他模型的关系

- 与 `PluginActiveSnapshot`：消费其中 tools/hooks/providers 的当前子集
- 与 `PromptDeclaration`：直接复用既有 prompt 注入链路

#### 校验规则与不变量

- `agent-runtime` 只消费这个 surface，不自行做资源发现
- 同一次 turn 内 `hook_snapshot_id` 必须稳定
- 不得包含 process-local 句柄或锁对象

#### 生命周期 / 状态变化

- `assembled -> bound_to_turn -> consumed -> discarded`

#### 映射关系

- `HostSessionSnapshot + PluginActiveSnapshot -> AgentRuntimeExecutionSurface`

### `HostSessionSnapshot`

- 用途：表示 `host-session` 持有的 session durable/read-model 视图
- 类型：视图模型
- 所属边界：`host-session`
- 来源：事件日志、投影、catalog、branch/fork 状态
- 去向：对外查询、`AgentRuntimeExecutionSurface` 组装、design/read model

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `session_id` | `string` | 是 | - | 非空 | 会话 id |
| `working_dir` | `string` | 是 | - | 非空 | 工作目录 |
| `event_log_revision` | `u64` | 是 | - | 单调递增 | durable 版本 |
| `read_model_revision` | `u64` | 是 | - | 单调递增 | 投影视图版本 |
| `lineage_parent_session_id` | `string` | 否 | `null` | 可为空 | 父 session |
| `active_turn_id` | `string` | 否 | `null` | 最多一个 | 当前活动 turn |
| `mode_id` | `string` | 否 | `null` | 可为空 | 当前 mode |
| `observe_snapshot` | `SessionObserveSnapshot` | 否 | `null` | 可为空 | 对外观察视图 |
| `terminal_snapshot` | `TurnTerminalSnapshot` | 否 | `null` | 可为空 | 最近终态视图 |

#### 与其他模型的关系

- 与 `AgentRuntimeExecutionSurface`：为 turn 组装提供 session 侧事实
- 与 `SessionObserveSnapshot` / `TurnTerminalSnapshot`：直接复用既有查询模型

#### 校验规则与不变量

- durable truth 以事件日志为准
- `active_turn_id` 最多一个
- 所有派生视图必须可由 durable truth 恢复

#### 生命周期 / 状态变化

- `created -> active -> compacted -> forked / archived / deleted`

#### 映射关系

- `event log -> HostSessionSnapshot -> query surface / AgentRuntimeExecutionSurface`

### `SubRunHandle`

- 用途：描述父 turn 与 child session/agent 之间的 durable 协作关系
- 类型：关系模型
- 所属边界：`host-session`
- 来源：父 session 发起子 agent、事件日志恢复、sub-run 生命周期推进
- 去向：query/read model、协作取消、结果投递、observability

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `subrun_id` | `string` | 是 | - | 全局唯一 | 子运行 id |
| `parent_session_id` | `string` | 是 | - | 非空 | 父 session |
| `parent_turn_id` | `string` | 是 | - | 非空 | 发起该子运行的父 turn |
| `child_session_id` | `string` | 是 | - | 非空 | 子 session |
| `child_agent_id` | `string` | 是 | - | 非空 | 子 agent |
| `status` | `"queued" \| "running" \| "succeeded" \| "failed" \| "cancelled"` | 是 | - | 枚举 | 当前状态 |
| `input_delivery_state` | `"pending" \| "delivered" \| "acknowledged"` | 是 | `pending` | 枚举 | 子输入投递状态 |
| `result_delivery_state` | `"pending" \| "delivered" \| "dropped"` | 是 | `pending` | 枚举 | 结果回传状态 |
| `spawn_reason` | `string` | 否 | `null` | 可为空 | 发起原因或能力说明 |
| `started_at_ms` | `u64` | 否 | `null` | 可为空 | 启动时间 |
| `finished_at_ms` | `u64` | 否 | `null` | 可为空 | 结束时间 |

#### 与其他模型的关系

- 与 `HostSessionSnapshot`：父/子 session 的 durable truth 都由 host 持有
- 与 `InputQueueProjection`：输入投递和回传依赖 queue read model

#### 校验规则与不变量

- 一个 `SubRunHandle` 只能绑定一个父 turn 和一个 child session
- child session 仍然是完整 session，因此必须满足“一个 session 即一个 agent”
- durable truth 以事件日志为准，`SubRunHandle` 必须可由事件恢复

#### 生命周期 / 状态变化

- `queued -> running -> succeeded / failed / cancelled`

#### 映射关系

- `spawn request -> SubRunHandle -> query/cancel/result-delivery`

### `InputQueueProjection`

- 用途：描述某个 session/agent 当前待处理输入的 read model，包括协作输入投递状态
- 类型：视图模型
- 所属边界：`host-session`
- 来源：事件日志、输入入队/出队、sub-run 投递
- 去向：host 调度、协作恢复、观测与调试

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `session_id` | `string` | 是 | - | 非空 | 所属 session |
| `queue_depth` | `u32` | 是 | `0` | `>= 0` | 当前队列长度 |
| `pending_input_ids` | `string[]` | 是 | `[]` | 保持顺序 | 待处理输入 id 列表 |
| `head_input_kind` | `"user" \| "parent_subrun" \| "follow_up"` | 否 | `null` | 枚举 | 队头输入类型 |
| `last_enqueued_input_id` | `string` | 否 | `null` | 可为空 | 最近入队输入 |
| `last_delivered_input_id` | `string` | 否 | `null` | 可为空 | 最近已投递输入 |
| `blocked_by_subrun_id` | `string` | 否 | `null` | 可为空 | 当前阻塞此 queue 的 subrun |
| `updated_at_ms` | `u64` | 是 | - | 单调时间戳 | 最近更新时间 |

#### 与其他模型的关系

- 与 `SubRunHandle`：sub-run 输入投递与结果回传会改变队列视图
- 与 `AgentRuntimeExecutionSurface`：只有当 host 从 queue 中取出输入后，runtime 才会消费对应 turn

#### 校验规则与不变量

- 输入队列属于 session durable truth 的派生读模型，而不是 runtime 内部缓存
- 任意一次服务重启后，都必须能通过事件日志恢复 `InputQueueProjection`
- runtime 不得直接突变 queue，所有变更都经由 `host-session`

#### 生命周期 / 状态变化

- `empty -> pending -> delivering -> settled`

#### 映射关系

- `input events + subrun delivery events -> InputQueueProjection -> host dispatch`

## 兼容性与迁移

- 本次是明确的 breaking change：
  - 旧 `application::App` 不再保留
  - 旧 `kernel` crate 不再保留
  - 旧 `core` 作为“大而全 DTO/trait 仓库”的定位不再保留
  - 旧 monolith `session-runtime` 对外 surface 不再作为长期正式边界
  - 旧窄版 `PluginManifest` / `core::HookInput` / `HookOutcome` 不足以表达新系统，需要迁移到新的 descriptor / envelope / effect 模型
- 不保留长期兼容 DTO 壳层：
  - `PluginDescriptor` 直接成为 `plugin-host` 的正式插件描述模型
  - `HookDescriptor + HookEventEnvelope + HookEffect` 直接成为新的 hooks 边界模型
  - owner 专属 DTO 直接迁回 owner crate，不保留 `core` 中转层
- 迁移顺序建议：
  - 先引入新模型并仅在新 crate 内消费
  - 再迁移旧 owner 的实现
  - 最后删除旧的 `application`、`kernel` 与 monolith `session-runtime`
- `SubRunHandle`、`InputQueueProjection` 与协作 executor 合同优先通过 `host-session` owner bridge 对外暴露；`ChildAgentRef`、`ChildSessionNode`、`ChildSessionLineageKind` 暂不迁出 `core`，直到 durable event wire schema 可以独立表达该嵌入结构。

## 复用说明

- 继续复用的现有模型：
  - `CapabilitySpec`
  - `PromptDeclaration`
  - `PromptGovernanceContext`
  - `SessionObserveSnapshot`
  - `TurnTerminalSnapshot`
- 复用理由：
  - 它们已经是纯数据模型，且能直接服务新边界
  - 特别是 prompt augment 不需要新建平行 DTO，沿用 `PromptDeclaration` 更干净
- 不继续复用的现有模型：
  - 旧 `PluginManifest`：贡献面太窄
  - 旧 `HookInput` / `HookOutcome`：事件面和 effect 面都太窄，只覆盖 tool/compact
  - 旧 `ports.rs`、`projection`、`session_catalog`、`workflow`、`mode` 中的大量 owner 模型：它们不适合继续保留在 `core`

## 补充贡献描述模型

以下贡献描述模型作为 `PluginDescriptor` 的正式子结构存在，由 `plugin-host` 统一校验与聚合，不再拥有独立的长期发现合同，也不再进入 `core`。

这些模型在本 change 中先锁定关键字段、约束与 owner 归属；完整字段表在实现 PR 中结合现有代码与 wire 需求定稿。

### `ProviderDescriptor`

- 用途：描述 provider 贡献与模型目录
- 关键字段：
  - `provider_id`
  - `display_name`
  - `api_kind`
  - `base_url`
  - `auth_scheme`
  - `models`
  - `timeout_ms`
  - `retry_policy`
  - `capabilities`
- 约束：
  - `provider_id` 在一个 active snapshot 中唯一
  - `models` 必须是纯数据目录，不得夹带运行时句柄

### `ResourceDescriptor`

- 用途：描述技能、模板、主题、命令目录之外的原始资源入口
- 关键字段：
  - `resource_id`
  - `kind`
  - `locator`
  - `scope`
  - `watch_mode`
  - `visibility`
  - `metadata`
- 约束：
  - `locator` 必须可解析
  - `kind` 只能落在正式注册的资源种类中

### `CommandDescriptor`

- 用途：描述 slash/命令能力
- 关键字段：
  - `command_id`
  - `display_name`
  - `description`
  - `argument_schema`
  - `entry_ref`
  - `permission_profile`
  - `interaction_mode`
- 约束：
  - `entry_ref` 必须指向可执行入口
  - `argument_schema` 必须可序列化、可校验

### `ThemeDescriptor`

- 用途：描述主题贡献
- 关键字段：
  - `theme_id`
  - `display_name`
  - `extends`
  - `tokens`
  - `metadata`
- 约束：
  - `tokens` 必须是纯数据 token 集
  - `extends` 若存在，必须引用当前 snapshot 中可解析的主题

### `PromptDescriptor`

- 用途：描述 prompt 模板贡献
- 关键字段：
  - `prompt_id`
  - `display_name`
  - `description`
  - `argument_schema`
  - `scope`
  - `body`
  - `metadata`
- 约束：
  - `body` 必须可安全渲染
  - `scope` 必须落在正式 prompt 作用域中

### `SkillDescriptor`

- 用途：描述 skill 贡献
- 关键字段：
  - `skill_id`
  - `display_name`
  - `description`
  - `allowed_tools`
  - `entry_ref`
  - `lazy_load`
  - `metadata`
- 约束：
  - `allowed_tools` 只能引用当前 snapshot 中存在的工具
  - `entry_ref` 必须指向可加载内容

## 未决问题

无。
