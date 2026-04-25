## 调研目标

本 change 聚焦一个更具体的问题：现有 hooks 总线、plugin descriptor、agent-runtime hook port 已经出现，但生产路径没有真实 handler；后续如果想只保留干净的 `agent-runtime` loop 和 `context-window` compact 核心，就必须把 plan mode、权限拦截、协作工具、commands/resources 等能力迁到 builtin plugin，并通过统一 hooks 总线接入。

## 当前现状

### 相关代码与模块

- `crates/plugin-host/src/hooks.rs` 已有 `dispatch_hook_bus()`，支持六种 dispatch mode 和 11 个正式事件，但 `HookBusStep` 当前携带预计算 `effect`，更像测试用 effect interpreter，而不是真实 handler executor。
- `crates/agent-runtime/src/hook_dispatch.rs` 定义 `HookDispatcher` port，`crates/agent-runtime/src/loop.rs` 在 turn/tool/provider 生命周期中有调用点，但 server 生产路径没有注入真实 dispatcher。
- `crates/server/src/session_runtime_port_adapter.rs` 构造 `TurnInput` 时只写入 `hook_snapshot_id: "server-owned"`，没有 `.with_hook_dispatcher(...)`。
- `crates/plugin-host/src/descriptor.rs` 的 `HookDescriptor` 只有 `hook_id` 和 `event`，缺少执行入口、优先级、失败策略、dispatch mode 和 schema。
- `crates/protocol/src/plugin/messages.rs` 当前没有 external plugin hook dispatch/result 消息。
- `crates/server/src/bootstrap/capabilities.rs::build_core_tool_invokers()` 仍直接注册 `EnterPlanModeTool`、`ExitPlanModeTool`、`UpsertSessionPlanTool` 等规划能力。
- `crates/server/src/bootstrap/runtime.rs::build_server_plugin_contribution_descriptors()` 已经把 provider、modes、composer command、core tools、MCP tools、collaboration tools 包装成 descriptor，但真实执行器仍由 server 另一路注册，存在双事实源。
- `crates/context-window/src/compaction.rs::auto_compact()` 是 compact 核心算法；它应保留为核心能力，而不是整体插件化。
- `crates/governance-contract/src/policy.rs` 有 `PolicyEngine`、`PolicyVerdict`、`AllowAllPolicyEngine` 等类型，但尚未形成 runtime tool/provider 检查点的真实权限执行链。
- `crates/core/src/hook.rs` 中旧 `HookEvent` 和旧 context 类型只有本文件定义与 `core/src/lib.rs` re-export，未发现 crate 外生产引用；同一文件中的 `HookEventKey` 仍被 agent-runtime、plugin-host、host-session、runtime-contract 使用，不能删除整个文件。
- `crates/server/src/governance_surface/`、`server/src/mode/compiler.rs`、`server/src/mode/catalog.rs`、`server/src/mode_catalog_service.rs`、`server/src/mode/validator.rs`、`server/src/governance_service.rs` 均有活跃消费者，不能作为“死代码”清理。

### 相关接口与能力

- 可复用：`agent-runtime` 已经通过 trait port 依赖 hooks，这是正确方向。
- 可复用：`plugin-host` 已有 active snapshot、descriptor、reload staging、builtin capability executor 相关模式。
- 缺口：hook descriptor 无法绑定 handler，external 协议无 hook 消息，builtin hook 无 executor registry。
- 缺口：runtime hook payload 太泛，tool call 无工具名/参数/result/capability spec/mode 信息，不能承载权限决策。

### pi-mono 参考

`D:\GitObjectsOwn\pi-mono` 的可借鉴点不是照搬实现，而是边界方式：

- `packages/agent/src/agent-loop.ts` 中 loop 只消费 `beforeToolCall`、`afterToolCall`、`transformContext` 等 callbacks。
- tool 前置拦截可以把某个 tool call 转换为 error tool result，而不是杀死整个 agent loop。
- `packages/coding-agent/src/core/extensions/runner.ts` 中 extension runner 按事件提供 `emitInput`、`emitToolCall`、`emitToolResult`、`emitBeforeProviderRequest` 等 domain-specific 方法。
- `packages/coding-agent/src/core/agent-session.ts` 在 session 层绑定 extension runner；reload 换 runner，不改底层 loop。

## 关键发现

### 发现 1：hook 引擎存在但未接入生产路径

- 事实：runtime 触发 hook port，plugin-host 有 bus engine，但 server 没有注入真实 dispatcher。
- 证据：`session_runtime_port_adapter.rs` 仅设置 `hook_snapshot_id`；`dispatch_hook_bus()` 没有生产调用链。
- 影响：hooks 不能支撑 plan mode、权限、provider request、compact 定制。
- 可复用点：保留 runtime port，在 plugin-host/server 侧实现真实 dispatcher。

### 发现 2：当前 tool_call hook 不适合权限系统

- 事实：payload 缺少工具上下文，`Block` 会变成 turn/tool dispatch error。
- 证据：`dispatch_runtime_hook()` 只发送 agent/stepIndex/messageCount；`execute_tool_calls()` 遇到 hook 终止直接返回 error。
- 影响：权限拒绝会表现为系统错误，无法只拒绝单个工具。
- 可复用点：采用 `BlockToolResult` typed effect，把拒绝转换成失败 tool result。

### 发现 3：tool_result hook 时机太晚

- 事实：当前先 `record_tool_result()` 再 dispatch `ToolResult`。
- 影响：未来即使有 `OverrideToolResult`，也无法修改持久化结果或模型上下文。
- 可复用点：把 `tool_result` hook 移到记录前。

### 发现 4：plan mode 切换必须在 turn 编译前

- 事实：capability router 和 governance envelope 在 runtime turn 前已编译。
- 影响：`turn_start` hook 切 plan mode 对当前 turn 太晚。
- 可复用点：通过 `host-session` 的 `input` hook 或正式 mode transition 入口，在 turn acceptance 前切换。

### 发现 5：compact 应保留核心算法，只开放 hook 定制

- 事实：`context-window` 是 compact owner。
- 影响：把 compact 算法外置成插件会破坏上下文恢复和请求整形稳定性。
- 可复用点：compact command 作为 builtin composer plugin，`session_before_compact` hook 定制输入/取消/外部摘要。

### 发现 6：清理必须分层，活跃 governance 路径不能提前删除

- 事实：旧 `core::hook` DTO 接近死代码，但 server governance、mode 编译/目录、mode catalog service 和 governance-contract 仍有多处生产引用。
- 证据：`rg` 显示旧 `HookEvent` / context 类型没有 crate 外引用；`governance_surface`、`compile_mode_envelope`、`builtin_mode_specs`、`governance_service` 有 server/adapter/contract 消费者。
- 影响：可以尽早删除旧 hook DTO 降低噪音，但不能提前删除 `governance-contract` 或 server governance 栈，否则会切断 reload、mode transition、child agent surface、provider prompt assembly 等路径。
- 可复用点：cleanup 应写成 gated migration：先接 hook dispatcher，再建立 builtin permission/planning plugin，再迁移 consumers，最后删除旧文件。

## 可选方案比较

| 方案 | 优点 | 风险 | 结论 |
| --- | --- | --- | --- |
| 在 server/runtime 继续加特判 | 短期快 | plugin-first 失败，双事实源扩大 | 不采用 |
| 让 agent-runtime 直接依赖 plugin-host | 接线直观 | 破坏 owner 边界 | 不采用 |
| hook 合同下沉到 contract，plugin-host 绑定执行器，server 注入 runtime/host-session | 边界清晰，可分阶段迁移 | 需要补类型、协议、测试 | 推荐 |

## 结论

- 推荐方向：用 `plugin-host` active snapshot 承载 executable hook bindings；runtime/host-session 只依赖 hook dispatch contract；plan/permission/collaboration/composer 通过 builtin plugin 迁移。
- 暂不采用：runtime `turn_start` 切 plan mode、generic `Block` 做工具权限拒绝、compact 核心插件化、runtime 直接依赖 plugin-host。
- 清理策略：立即清理仅限旧 `core::hook` DTO；其余 governance/mode/plan 文件必须等待 hook 替代路径、调用点迁移和测试通过。

## 未决问题

- hook contract 放入 `runtime-contract` 还是新增 `hook-contract`。第一阶段建议先放 `runtime-contract`，避免新增 crate。
- external hook schema 第一阶段是否支持 inline JSON Schema。建议先支持宿主内置 schema 名称。
