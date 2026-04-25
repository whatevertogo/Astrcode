## 背景

现有系统的 hooks、plugin-host、mode、policy 都有雏形，但还没有闭合成一条生产执行链。`plugin-host` 有 hook bus engine，`agent-runtime` 有 `HookDispatcher` port，server 生产路径却没有注入真实 dispatcher；builtin plugin 多数仍只是 descriptor 包装，真实执行器注册和 governance/mode 逻辑仍散落在 server bootstrap、governance surface、mode compiler 和 adapter-tools 中。

如果目标是后续只保留干净的 `agent-runtime` loop 与 `context-window` compact 核心，其他能力通过 builtin plugin + hooks 总线提供，那么本 change 需要先打通 executable hook binding，再迁移 planning / permission / composer / collaboration 等能力。否则会继续扩大 descriptor 与执行器两套事实源。

DeepSeek 计划中“按 pi-mono 的 `pi.on(event, handler)` 学习函数式扩展 API”值得借鉴；但不能照搬其中的 `HookBusStep { effect }` 生产模型、`plugin-host -> agent-runtime` 直接依赖、或提前删除活跃 governance 栈的部分。

## 目标

- 让 hooks 成为真实可执行总线，builtin 和 external plugin 都能注册 handler。
- 保持 `agent-runtime` 干净，只通过 contract/adapter 注入的 hook dispatcher 消费 hook 结果，不依赖 `plugin-host`。
- 让 builtin hook 作者能用函数注册风格表达 `on_tool_call`、`on_before_provider_request`、`on_tool_result` 等 handler，trait object 仅作为内部擦除层。
- 让 plan mode 入口、权限拦截、provider request 策略、tool result 修饰、compact 定制通过 hooks 或 builtin plugin 生效。
- 保留 `context-window` compact 核心算法，不把 compact 算法整体插件化。
- 把 planning、permission、composer、collaboration、core tools 逐步迁移成 builtin plugin contribution。
- 建立明确 cleanup gate：已验证无外部引用的旧 hook DTO 可先删；活跃 governance/mode/server 路径必须等替代路径验证通过后再删。

## 非目标

- 不重写 agent loop、provider streaming、conversation projection 或 JSONL event store。
- 不实现完整多语言插件 SDK；本次至少定义 hook 协议和 Rust 侧 builtin executor 合同。
- 不设计 OOP 风格的插件生命周期框架。
- 不让 runtime `turn_start` hook 改变当前 turn envelope。
- 不在本 change 第一阶段删除整个 `governance-contract`；先把 `PolicyVerdict` / `ApprovalRequest` 等策略结果接入 hook effect，确认替代路径稳定后再收缩或删除。
- 不在 hook handler 中提供直接写 durable truth 的 `append_entry()` / mutable session handle；handler 只能返回 effect，由 owner 应用。
- 不保留旧硬编码注册路径作为长期兼容层，但删除必须遵守 cleanup gate。

## 变更内容

### 新增基础设施

- 新增 hook contract：将正式 hook request/outcome/payload/effect 放到 `runtime-contract` hooks 模块，或在实现早期由 server adapter 桥接到该模型，避免 `plugin-host -> agent-runtime` 横向依赖。
- 扩展 executable `HookDescriptor`：包含 event、stage、dispatch mode、failure policy、priority、entry ref、payload/effect schema、origin plugin。
- 新增 `HookBinding`：active snapshot 中的可执行 hook 条目，包含 registration + builtin/external executor ref。生产路径不再保存预计算 effect。
- 新增 builtin hook registry：支持 `registry.on_tool_call(...)`、`registry.on_before_provider_request(...)` 等函数式注册 helper；内部可擦除为 `BuiltinHookExecutor` trait object。
- 新增受限 `HookContext`：只提供 typed event context、只读状态视图和受限 action request 能力；任何 session mutation 必须通过 effect 回到 owner。
- 新增 external hook protocol message：external plugin 可接收 `dispatch_hook` 并返回 typed effects。

### 修改已有组件

- `dispatch_hook_bus()` 从同步 effect interpreter 改为 async handler dispatcher；如果保留 `HookBusStep` 名称，它必须表示 handler binding，而不是 `{ registration, effect }`。
- runtime hook effect 语义合并到 contract 层 typed effect，不以 plugin-host 当前 enum 作为长期唯一事实源。
- `tool_call` 普通拒绝生成失败 tool result；`tool_result` 在持久化前运行；provider/context hooks 使用 typed payload/effect。
- plan/permission 路径：plan mode 自动切换走 `host-session` input hook或正式 mode transition 入口；权限走 `tool_call` / `before_provider_request` hook。
- compact command 仍是 command contribution；`session_before_compact` 只负责执行前取消、改写输入或提供摘要，不替代 command 本身。

### 分阶段 cleanup

**可先清理：**

- `core/src/hook.rs` 中旧 `HookEvent` enum 与旧 context 类型：`ToolHookContext`、`ToolHookResultContext`、`CompactionHookContext`、`CompactionHookResultContext`。只能删除这些旧类型和 re-export，不能删除仍被使用的 `HookEventKey`。

**hooks 接线并验证后再清理：**

- `governance-contract/src/policy.rs` 的 executable policy 路径：先用 builtin permission hook 替代 `PolicyEngine` 执行点；`ModeId`、`GovernanceModeSpec`、`SystemPromptBlock` 等仍被多 crate 使用，不能一刀切。
- `server/src/governance_service.rs`、`server/src/bootstrap/governance.rs`、`server/src/lifecycle/governance.rs`、`server/src/governance_surface/`。
- `server/src/mode/compiler.rs`、`server/src/mode/catalog.rs`、`server/src/mode/builtin_prompts.rs`、`server/src/mode_catalog_service.rs`、`server/src/mode/validator.rs`。
- `adapter-tools/src/builtin_tools/enter_plan_mode.rs`、`exit_plan_mode.rs`、`mode_transition.rs`：迁移为 builtin planning plugin 后删除直接注册路径；session plan 编辑工具可继续作为 tool contribution 存在。

**不纳入本 change 必要完成条件：**

- 删除或拆分整个 `runtime-contract`。
- 删除 `host-session/src/workflow.rs` 或 `host-session/src/composer.rs`。
- 实现 pi-mono 的 inter-plugin event bus。

## 能力变更

### 新增能力

- `plugin-hook-execution`: 定义 plugin-host 如何把 hook descriptor 绑定到 builtin/external handler 并执行。
- `builtin-runtime-plugins`: 定义 runtime 周边能力如何通过 builtin plugin contribution 装配。
- `typed-runtime-hook-effects`: 定义按事件约束的 payload/effect，避免裸 JSON effect 扩散。
- `legacy-governance-cleanup`: 定义旧 hook/governance/mode/plan 代码的删除前置条件、迁移顺序和验证门槛。

### 修改能力

- `lifecycle-hooks-platform`: 增加真实 handler execution、effect validation 和 owner 应用约束。
- `plugin-host-runtime`: active snapshot 必须包含 descriptor 与 executable binding。
- `agent-runtime-core`: runtime hook payload/effect 和 tool denial/result override 语义收紧。
- `governance-mode-system`: plan mode 入口在 turn envelope 编译前完成。

## 影响范围

- `crates/plugin-host`: descriptor、snapshot、hook dispatcher core、builtin hook executor registry、function-style helper、diagnostics。
- `crates/agent-runtime`: hook contract、tool call/result/provider hook 应用语义。
- `crates/host-session`: input/session_before_compact/model_select owner hooks。
- `crates/server`: hook dispatcher adapter 注入、snapshot id、builtin plugin 迁移、governance/mode cleanup gate。
- `crates/protocol`: plugin hook dispatch/result DTO。
- `crates/governance-contract`: policy verdict 到 hook effect 的映射；第一阶段不直接删除整个 crate。
- `crates/core/src/hook.rs`: 删除旧 hook DTO，保留或迁移 `HookEventKey`。

## 约束与风险

- hook effect 必须强类型化；裸 `serde_json::Value` 只能存在于协议边界。
- 权限类 hook 默认 fail-closed；观测类 hook 可 report-only。
- session durable truth 只能由 owner 写入，hook 只能返回 effect。
- reload 必须保持 snapshot 一致性，进行中的 turn 使用旧 snapshot。
- `HookContext` 不能成为新的万能 service locator；它只提供 event-specific typed context、只读视图和受限 action request。
- 活跃 governance/mode/server 文件必须通过 `rg` + 测试验证后再删，不能按文件名直接清理。
