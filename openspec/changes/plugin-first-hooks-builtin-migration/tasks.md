## 1. 合同与 descriptor

- [x] 1.1 在 `crates/runtime-contract/src` 新增 hooks 合同模块，定义 typed request/outcome/payload/effect。
- [x] 1.2 调整 `crates/agent-runtime/src/hook_dispatch.rs`，消费或 re-export contract 层 hook 类型。
- [x] 1.3 扩展 `crates/plugin-host/src/descriptor.rs::HookDescriptor` 字段。
- [x] 1.4 更新 `crates/plugin-host/src/loader.rs` 和 handshake/manifest 映射（外部插件当前不通过 manifest 贡献 hooks，loader 保持 `hooks: Vec::new()`，无需变更）。
- [x] 1.5 更新旧 hook descriptor 测试 fixture。
- [x] 1.6 用 `rg` 固化旧 `core::hook` DTO 的引用现状，确认只删除旧 `HookEvent` / context 类型，不删除仍被使用的 `HookEventKey`。

## 2. plugin-host hook binding

- [x] 2.1 新增 `BuiltinHookExecutor` trait 和 registry，trait 作为内部擦除层，不作为 builtin plugin 作者必须手写的主要 API。
- [x] 2.2 在 snapshot staging 中生成 `HookBinding`，校验 descriptor 与 executor/backend handler 对应；生产 snapshot 不得保存预计算 `HookBusStep { effect }`。
- [x] 2.3 重写 `crates/plugin-host/src/hooks.rs` 生产路径，执行 handler 而不是消费静态 effect。
- [x] 2.4 实现 priority、dispatch mode、failure policy、effect validation（已内嵌在 `dispatch_hooks` 函数中）。
- [x] 2.5 在 `BuiltinHookRegistry` 增加函数式注册 helper，例如 `on_input`、`on_tool_call`、`on_tool_result`、`on_before_provider_request`，handler 接收 typed context 并返回 typed effect。
- [x] 2.6 新增受限 `HookContext`，只暴露 typed metadata、只读 host view、取消状态和受限 action request，不暴露 `EventStore` / mutable session state / snapshot mutation。
- [x] 2.7 补充 plugin-host 单元测试，覆盖 trait executor、函数式 helper、受限 HookContext 和 invalid effect validation。

## 3. external hook protocol

- [x] 3.1 在 `crates/protocol/src/plugin/messages.rs` 增加 `dispatch_hook` / `hook_result` DTO。
- [ ] 3.2 更新 plugin backend，把 external hook binding 转成协议消息（已有 `transport.dispatch_hook()` 和 `handle.dispatch_hook()` scaffold；`dispatch_hooks()` 尚未接入 external runtime handle）。
- [ ] 3.3 校验 external response 的 correlation id 和 effect 集合（transport 层已校验 correlation_id；`HookEffectWire -> HookEffect` 映射未接入生产路径）。
- [ ] 3.4 补充协议/backend 测试（低优先级，依赖 external plugin 集成）。

## 4. runtime hook 语义

- [x] 4.1 在 `agent-runtime` 为正式事件构造 typed payload（`dispatch_typed_hook` 支持 typed `HookEventPayload`）。
- [x] 4.2 修改 `tool_call` 流程，支持 `MutateToolArgs`、`BlockToolResult`、`RequireApproval`、`CancelTurn`。
- [x] 4.3 把 `tool_result` hook 移到 `record_tool_result()` 前。
- [x] 4.4 接入 `before_provider_request` 修改/拒绝 effect（`DenyProviderRequest` 支持完成 turn，`ModifyProviderRequest` 基础集成）。
- [x] 4.5 补充 runtime 单元测试（新增 `block_tool_call_effect_produces_failed_tool_result` 测试，验证 BlockToolResult 效果）。

## 5. host-session owner hooks

- [ ] 5.1 在 input 接收路径加入 `input` hook，运行在 turn acceptance 前（需要在 `workflow.rs` / `execution_surface.rs` 中找到 input 接收路径并插入 HookDispatch 调用）。
- [ ] 5.2 实现 `TransformInput`、`HandledInput`、`SwitchMode` effect（需要 host-session 的 input 处理逻辑支持这些 effect 的输出）。
- [ ] 5.3 在 compact 入口加入 `session_before_compact` hook（需要在 `compaction.rs` 中找到 compact 触发路径）。
- [ ] 5.4 在 model select 入口加入 `model_select` hook（`model_selection.rs` 已改为 typed `HookDispatch` port，但生产入口尚未注入 dispatcher）。
- [ ] 5.5 补充 host-session replay 和 compact 测试。

## 6. server 接线

- [x] 6.1 构建 server adapter，将 plugin-host dispatch core 包装成 `agent-runtime` 可消费的 hook dispatcher，并注入 `TurnInput::with_hook_dispatcher(...)`，避免 `plugin-host -> agent-runtime` 依赖（已创建 `PluginHostHookDispatcher`；server bootstrap 使用 active snapshot 的 `hook_bindings` 构造并注入）。
- [x] 6.2 用真实 active snapshot id 替换 `"server-owned"`（server bootstrap 将 `plugin_host_reload.snapshot.snapshot_id` 传入 session runtime）。
- [ ] 6.3 通过 server adapter 向 host-session 注入 owner hook dispatcher（host-session port 已 typed；生产注入尚未完成）。
- [ ] 6.4 更新 reload，保证 hook bindings 与 descriptors 原子替换。

## 7. builtin plugin 迁移

- [ ] 7.1 新增 builtin planning plugin，贡献 plan tools、mode descriptor、input hook。
- [ ] 7.2 从 `build_core_tool_invokers()` 移除 planning tools 直接注册。
- [ ] 7.3 新增 builtin permission plugin，复用现有 `governance-contract` 的 `PolicyVerdict` / `ApprovalRequest` 类型并映射为 `HookEffect`，本阶段不直接删除整个 crate。
- [ ] 7.4 迁移 composer compact command，compact core 仍调用 `context-window`。
- [ ] 7.5 修正 collaboration descriptor 与真实 executor 名称一致。
- [ ] 7.6 分批迁移 core tools，每迁移一个能力就删除对应旧事实源。

## 8. 清理与验证

- [x] 8.1 隔离生产不用的静态 `HookBusStep { effect }`（已添加 doc 注释标注为 legacy，仅测试使用）。
- [ ] 8.2 删除 stale collaboration descriptor（当前 `builtin_collaboration_tools_descriptor` 仍被 server bootstrap 使用，暂不可删）。
- [x] 8.3 删除旧 `core::hook` DTO：`HookEvent`、`ToolHookContext`、`ToolHookResultContext`、`CompactionHookContext`、`CompactionHookResultContext` 及其 re-export；保留 `HookEventKey`。
- [x] 8.4 建立 governance/mode cleanup 迁移清单（见下方备注）。
- [x] 8.5 在 hook/builtin plugin 替代路径验证通过前，不删除 `governance-contract`、server governance surface、mode compiler/catalog 或 plan-mode adapter tools。（约束已理解）
- [x] 8.6 更新 `PROJECT_ARCHITECTURE.md` 的 hook/builtin plugin 边界说明。
- [x] 8.7 运行 `cargo check --workspace`。
- [x] 8.8 运行 `node scripts/check-crate-boundaries.mjs`。
- [x] 8.9 运行单元测试（`cargo test --workspace --exclude astrcode --lib` 通过；相关链路覆盖 agent-runtime 23、plugin-host 100、host-session model_select 4）。
- [x] 8.10 运行 `cargo clippy --all-targets --all-features -- -D warnings`。
