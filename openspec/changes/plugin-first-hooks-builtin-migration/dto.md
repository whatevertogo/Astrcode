## 概览

本次涉及三类模型：

- plugin descriptor / snapshot 模型：扩展 `HookDescriptor`，新增 executable hook binding。
- protocol DTO：external plugin hook dispatch request/result。
- runtime/host 内部模型：typed `HookEventPayload`、`HookEffect`、`HookDispatchOutcome`。

现有 `CapabilitySpec`、`GovernanceModeSpec`、`PolicyVerdict`、`ApprovalRequest`、`ModeChanged` durable event 继续复用。

## 模型清单

### `HookDescriptor`

- 用途：描述 plugin 声明的 hook contribution。
- 类型：DTO / 描述符。
- 所属边界：plugin manifest / plugin-host descriptor。
- 来源：builtin descriptor、external manifest、handshake。
- 去向：`HookRegistration`。

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `hook_id` | `String` | 是 | - | active snapshot 内唯一 | hook 标识 |
| `event` | `HookEventKey` | 是 | - | 正式 hook catalog | 目标事件 |
| `stage` | `HookStage` | 是 | 由 event 推导 | 与 owner 匹配 | runtime/host/resource |
| `dispatch_mode` | `HookDispatchMode` | 是 | event 默认 | 与 event 兼容 | 合并语义 |
| `failure_policy` | `HookFailurePolicy` | 是 | event 默认 | 权限类默认 fail-closed | 失败策略 |
| `priority` | `i32` | 否 | `0` | 同优先级按 hook_id 排序 | 执行顺序 |
| `entry_ref` | `String` | 是 | - | builtin 或 external handler ref | 执行入口 |
| `input_schema` | `String` | 否 | event 默认 schema | 不得扩大 payload 语义 | 输入 schema id |
| `effect_schema` | `String` | 否 | event 默认 schema | 不得允许 event 不支持的 effect | 输出 schema id |

#### 校验规则与不变量

- descriptor 必须能解析到 executor 或 backend handler。
- `effect_schema` 只能收窄事件允许 effect 集合。

#### 映射关系

- `PluginManifestHook -> HookDescriptor -> HookRegistration -> HookBinding`。

### `HookBinding`

- 用途：active snapshot 中可执行 hook 条目。
- 类型：内部模型。
- 所属边界：plugin-host。
- 来源：descriptor staging。
- 去向：hook dispatcher。

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `plugin_id` | `String` | 是 | - | active plugin | 所属 plugin |
| `hook_id` | `String` | 是 | - | active snapshot 内唯一 | hook 标识 |
| `event` | `HookEventKey` | 是 | - | 正式事件 | 目标事件 |
| `dispatch_mode` | `HookDispatchMode` | 是 | - | 与 event 兼容 | 合并语义 |
| `failure_policy` | `HookFailurePolicy` | 是 | - | 与风险等级兼容 | 错误策略 |
| `priority` | `i32` | 是 | `0` | 稳定排序 | 执行顺序 |
| `executor` | `HookExecutorRef` | 是 | - | builtin/external 二选一 | 可调用执行器 |
| `snapshot_id` | `String` | 是 | - | 与 active snapshot 一致 | snapshot 一致性 |

#### 校验规则与不变量

- binding 不包含预计算 effect。
- commit 后不可变；reload 创建新 binding。

### `BuiltinHookRegistration`

- 用途：描述内置 hook handler 的注册结果。
- 类型：内部 registry model。
- 所属边界：plugin-host builtin hook registry。
- 来源：函数式 registration helper 或显式 executor registration。
- 去向：`HookBinding.executor`。

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `entry_ref` | `String` | 是 | - | `builtin://hooks/<id>` | descriptor 绑定 key |
| `event` | `HookEventKey` | 是 | - | 与 helper 类型一致 | 支持事件 |
| `handler` | `Arc<dyn BuiltinHookExecutor>` | 是 | - | `Send + Sync` | 内部擦除后的 executor |
| `context_kind` | `HookEventKey` | 是 | - | 与 event 一致 | 用于校验 typed helper 没有错配 |

#### 校验规则与不变量

- 函数式 helper 必须在注册时把 handler 约束到对应 typed context。
- registry 内部可以使用 trait object，但 builtin plugin 作者不需要为每个简单 hook 手写 struct。

### `HookContext`

- 用途：hook handler 执行时的受限上下文。
- 类型：内部 invocation context。
- 所属边界：plugin-host dispatch core / server adapter。
- 来源：hook owner 构造的 typed payload 与 server 注入的只读宿主视图。
- 去向：builtin hook handler。

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `snapshot_id` | `String` | 是 | - | 当前 hook snapshot | snapshot 一致性 |
| `session_id` | `String` | 否 | `None` | session scoped 事件必填 | 会话标识 |
| `turn_id` | `String` | 否 | `None` | turn scoped 事件必填 | turn 标识 |
| `agent_id` | `String` | 否 | `None` | agent/runtime 事件必填 | agent 标识 |
| `current_mode` | `ModeId` | 否 | `None` | mode-aware 事件必填 | 当前 mode |
| `cancel_state` | `CancelStateView` | 否 | `None` | 只读 | 取消信号视图 |
| `host_view` | `HookHostView` | 否 | `None` | 只读 | 受限状态查询 |
| `action_sink` | `HookActionSink` | 否 | `None` | 只能提交 action request | 可选受限动作通道 |

#### 校验规则与不变量

- `HookContext` 不得暴露 `EventStore`、`SessionState`、锁、channel sender、mutable plugin snapshot 或任意 owner 内部 handle。
- `action_sink` 只能提交受限 action request，不能直接 append durable event；owner 必须验证 action 或 effect 后再应用。
- event-specific typed payload 仍是 handler 的主要输入；`HookContext` 只提供横切元数据和只读视图。

### `HookDispatchMessage`

- 用途：host 向 external plugin 发送 hook dispatch。
- 类型：协议 request DTO。
- 所属边界：`protocol` plugin messages。
- 来源：plugin-host。
- 去向：external backend。

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `type` | `"dispatch_hook"` | 是 | - | 固定值 | 消息类型 |
| `correlation_id` | `String` | 是 | - | 单次请求唯一 | 请求响应关联 |
| `snapshot_id` | `String` | 是 | - | 当前 snapshot | 一致性 |
| `plugin_id` | `String` | 是 | - | active plugin | 目标 plugin |
| `hook_id` | `String` | 是 | - | active hook | 目标 handler |
| `event` | `String` | 是 | - | 正式事件名 | 事件 key |
| `payload` | `serde_json::Value` | 是 | - | 符合 event schema | typed payload 的 wire 表示 |

### `HookDispatchResultMessage`

- 用途：external plugin 返回 hook 执行结果。
- 类型：协议 response DTO。
- 所属边界：`protocol` plugin messages。
- 来源：external backend。
- 去向：plugin-host validation。

#### 字段定义

| 字段 | 类型 | 必填 | 默认值 | 约束 | 说明 |
| --- | --- | --- | --- | --- | --- |
| `type` | `"hook_result"` | 是 | - | 固定值 | 消息类型 |
| `correlation_id` | `String` | 是 | - | 匹配 request | 请求响应关联 |
| `effects` | `Vec<HookEffectWire>` | 是 | `[]` | 属于 event 允许集合 | handler 输出 |
| `diagnostics` | `Vec<HookDiagnosticWire>` | 否 | `[]` | 不含秘密 | 诊断 |

### `HookEventPayload`

- 用途：内部 hook dispatch 输入。
- 类型：内部 typed enum。
- 所属边界：runtime/host/plugin owner 到 dispatcher。
- 来源：正式 hook owner。
- 去向：builtin executor 或 external protocol。

#### 字段定义

| 变体 | 关键字段 | Owner |
| --- | --- | --- |
| `Input` | `session_id`, `source`, `text`, `images`, `current_mode` | `host-session` |
| `BeforeProviderRequest` | `session_id`, `turn_id`, `provider_ref`, `model_ref`, `request`, `current_mode` | `agent-runtime` |
| `ToolCall` | `session_id`, `turn_id`, `agent_id`, `tool_call_id`, `tool_name`, `args`, `capability_spec`, `working_dir`, `current_mode`, `step_index` | `agent-runtime` |
| `ToolResult` | `session_id`, `turn_id`, `tool_call_id`, `tool_name`, `args`, `result`, `ok`, `current_mode` | `agent-runtime` |
| `SessionBeforeCompact` | `session_id`, `reason`, `messages`, `settings`, `current_mode` | `host-session` |
| `ResourcesDiscover` | `snapshot_id`, `cwd`, `reason` | `plugin-host` |
| `ModelSelect` | `session_id`, `current_model`, `candidate_model`, `reason` | `host-session` |

#### 校验规则与不变量

- payload 不包含 mutable owner handle。
- tool payload 必须携带 `CapabilitySpec`，权限 handler 不应反查另一份工具表。

### `HookEffect`

- 用途：hook handler 返回的受约束 effect。
- 类型：内部 typed enum / wire tagged JSON。
- 所属边界：dispatcher 到 owner。
- 来源：hook handler。
- 去向：owner application。

#### 字段定义

| 变体 | 允许事件 | 说明 |
| --- | --- | --- |
| `Continue` | 全部 | 继续 |
| `Diagnostic` | 全部 | 记录诊断 |
| `TransformInput` | `input` | 转换输入 |
| `HandledInput` | `input` | 不创建 turn |
| `SwitchMode` | `input` | 请求 mode transition |
| `ModifyProviderRequest` | `before_provider_request` | 改写 provider payload |
| `DenyProviderRequest` | `before_provider_request` | 阻止网络请求 |
| `MutateToolArgs` | `tool_call` | 修改工具参数 |
| `BlockToolResult` | `tool_call` | 拒绝单个工具并生成失败结果 |
| `RequireApproval` | `tool_call`, `before_provider_request` | 发起审批 |
| `OverrideToolResult` | `tool_result` | 修改最终工具结果 |
| `CancelTurn` | runtime 可取消事件 | 取消 turn |
| `CancelCompact` | `session_before_compact` | 取消 compact |
| `OverrideCompactInput` | `session_before_compact` | 修改 compact 输入 |
| `ProvideCompactSummary` | `session_before_compact` | 外部摘要 |
| `ResourcePath` | `resources_discover` | 贡献资源路径 |
| `ModelHint` | `model_select` | 模型建议 |
| `DenyModelSelect` | `model_select` | 拒绝模型切换 |

#### 校验规则与不变量

- effect 必须属于 event 允许集合。
- `BlockToolResult` 不能替代 `CancelTurn`。
- effect 不直接持久化；owner 应用后产生正式事件。

### `ExecutionSubmissionOutcome`

- 用途：描述 prompt submit 经 input hook 处理后的结果。
- 类型：内部 owner/port 合同。
- 所属边界：`runtime-contract` / server session submit 边界。
- 来源：`host-session` input hook owner 应用与 turn acceptance。
- 去向：server root/session prompt API、agent collaboration submit 调用点。

#### 变体定义

| 变体 | 字段 | 说明 |
| --- | --- | --- |
| `Accepted` | `ExecutionAccepted` | 已创建 turn，后续进入 runtime 执行 |
| `Handled` | `session_id`, `response` | input hook 已处理输入，不创建 turn，也没有 turn id |

#### 校验规则与不变量

- `Handled` 不得持有伪造 turn id。
- 只有 `Accepted` 能触发 runtime turn spawn。
- child/subagent submit 调用点如收到 `Handled`，必须显式处理或报错，不能当作 accepted turn。

## 兼容性与迁移

- BREAKING：旧 `HookDescriptor { hook_id, event }` 必须迁移。
- BREAKING：普通工具拒绝不再使用 generic `Block`，迁移为 `BlockToolResult`。
- BREAKING：`tool_result` hook 从记录后迁移到记录前。
- BREAKING：session submit port 从只返回 `ExecutionAccepted` 改为 `ExecutionSubmissionOutcome`，以表达 `HandledInput` 不创建 turn 的语义。
- external plugin 新增 `dispatch_hook` / `hook_result` 消息；旧协议没有 hook handler，不做兼容层。

## 复用说明

- 复用 `CapabilitySpec` 作为工具权限事实。
- 复用 `GovernanceModeSpec` 作为 builtin planning plugin 的 mode 贡献。
- 复用 `PolicyVerdict` / `ApprovalRequest`，但必须映射为 typed hook effect。
- 复用 durable `ModeChanged` 和 tool result events，hook effect 不直接落库。

## 未决问题

- `RequireApproval` 是否需要 hook-specific wrapper；建议优先复用 `ApprovalRequest` 并加 source metadata。
