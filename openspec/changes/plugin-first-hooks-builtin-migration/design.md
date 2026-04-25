## 背景与现状

当前 hooks 和 plugin-host 方向正确，但实现还停留在“descriptor 和测试引擎”阶段。runtime 有 hook port，plugin-host 有 dispatch engine，server 没有把两者接上；同时 planning/permission/collaboration/composer 等能力仍由 server 或 runtime 硬编码。

## 设计目标

- 打通 executable hooks：descriptor -> binding -> dispatcher -> owner effect application。
- 保持 `agent-runtime` 不依赖 `plugin-host`。
- 用 typed payload/effect 支撑 plan、permission、provider、tool result、compact 定制。
- 分阶段迁移 builtin 能力，删除旧双事实源。

## 非目标

- 不插件化 compact 核心算法。
- 不实现 mode-specific runtime loop。
- 不保留旧 hook 协议兼容层。

## 方案概览

整体分四层：

1. Contract 层定义 hook typed payload/effect 和 dispatcher request/outcome。
2. `plugin-host` 负责 descriptor 校验、executor/backend binding、active snapshot 和 dispatch。
3. `agent-runtime`、`host-session`、`plugin-host` 各自在正式 owner 点触发 hook，并只在 owner 内应用 effect。
4. planning、permission、composer、collaboration 迁移为 builtin plugin contribution。

内置插件的作者体验采用 pi-mono 式“函数注册”心智模型，但不照搬 TypeScript 实现。Rust 中可以用 registry helper 接收 async function/closure，内部再擦除为 trait object：

```rust
registry.on_tool_call("builtin-plan-mode.block-writes", |ctx| async move {
    // 返回 HookEffect::Continue / BlockToolResult / MutateToolArgs ...
});
```

这样保留 Rust 的类型边界，同时避免每个简单 hook 都需要手写独立 struct 和 impl。

DeepSeek 计划中提到的 `HookContext` 可以借鉴，但必须收紧：它不是 pi-mono `ExtensionContext` 的完整复制，也不是新的 service locator。它只提供 event-specific typed context、只读状态快照、取消信号读视图，以及少量受限 action request builder。任何会持久化或改变 runtime 的操作，都必须表现为 `HookEffect`，由 `host-session` / `agent-runtime` / `plugin-host` owner 应用。

## 关键决策

### 决策 1：hook dispatch 合同下沉到 contract 层

- 决策：把正式 hook request/outcome/effect 放到 `runtime-contract` hooks 模块，或实现时先用 server adapter 桥接但以 contract 模型为目标。
- 原因：避免 `plugin-host -> agent-runtime` 横向依赖。
- 备选方案：plugin-host 直接实现 agent-runtime 内部 trait。
- 为什么没选：破坏 owner 边界。

### 决策 2：普通工具拒绝生成失败 tool result

- 决策：`tool_call` permission denial 返回 `BlockToolResult`。
- 原因：拒绝一个工具不是 turn fatal error。
- 备选方案：继续使用 generic `Block`。
- 为什么没选：plan mode 权限拒绝会表现成系统错误。

### 决策 3：plan mode 切换放在 input 阶段

- 决策：plan mode 自动切换由 `host-session` input hook 或 mode transition 入口完成。
- 原因：runtime `turn_start` 已晚于 envelope 编译。
- 备选方案：turn_start hook 切换。
- 为什么没选：当前 turn 工具面不会同步变化。

### 决策 4：builtin hook API 使用函数注册风格

- 决策：`BuiltinHookRegistry` 对外提供 `on_input`、`on_tool_call`、`on_tool_result`、`on_before_provider_request` 等注册函数，handler 可以是 async closure；内部再封装为统一 executor。
- 原因：pi-mono 值得借鉴的是扩展作者不用实现复杂类层级，只需订阅事件并返回 effect。
- 备选方案：要求每个 hook 都定义一个实现 `BuiltinHookExecutor` 的 struct。
- 为什么没选：这会让简单的 plan/permission hook 变成样板代码，降低 builtin plugin 的可读性；trait 应作为内部抽象，而不是插件作者 API。

### 决策 5：HookBusStep 不作为生产 effect 容器

- 决策：生产路径使用 `HookBinding` / handler binding。如果保留 `HookBusStep` 这个名字，它必须被改造成 handler binding，而不是 `{ registration, effect }`。
- 原因：hook effect 依赖当前 payload、mode、tool args、session 状态和 failure policy，不能在 snapshot staging 时预先算好。
- 备选方案：在 `PluginHostReload` 中收集 `Vec<HookBusStep { registration, effect }>`。
- 为什么没选：这只能支持测试 fixture，无法表达权限、plan mode 或 provider request 动态裁决。

### 决策 6：cleanup 使用 gate，不按文件名批量删除

- 决策：旧 `core::hook` DTO 可以作为第一批清理；server governance、mode compiler/catalog、mode catalog service、governance-contract 必须等替代路径与测试通过后再删。
- 原因：代码检索显示这些 server/governance 路径仍有活跃消费者。
- 备选方案：按 DeepSeek 删除清单一次性删除。
- 为什么没选：会破坏 reload、mode transition、subagent governance surface 和 provider prompt assembly。

## 数据流 / 控制流 / 错误流

### Bootstrap / Reload

```text
builtin modules + external descriptors
  -> PluginDescriptor + executor registries
  -> plugin-host stage_candidate
  -> validate + bind handlers
  -> commit active snapshot
  -> server builds adapter around plugin-host dispatch core
  -> server injects dispatcher into host-session / agent-runtime
```

失败：descriptor 无效、entry_ref 不能绑定、external backend 不支持 handler，candidate 失败并保留旧 snapshot。

### Tool Call

```text
provider emits tool calls
  -> runtime builds ToolCall payload
  -> dispatcher executes hooks
  -> MutateToolArgs / BlockToolResult / RequireApproval / CancelTurn / Continue
  -> allowed tools execute
  -> ToolResult hook before record
  -> persist final result
```

### Input / Plan Mode

```text
user input
  -> host-session input hook
  -> TransformInput / HandledInput / SwitchMode / Continue
  -> mode transition durable event if needed
  -> compile envelope
  -> runtime turn
```

### Compact

```text
compact requested
  -> session_before_compact hook
  -> cancel / override input / provide summary / continue
  -> context-window compact core
```

## 与 DTO / Spec 的对应关系

- `HookDescriptor`、`HookBinding` 满足 `plugin-hook-execution`。
- `HookDispatchMessage`、`HookDispatchResultMessage` 满足 external protocol。
- `HookEventPayload`、`HookEffect` 满足 `typed-runtime-hook-effects`。
- builtin planning/permission/composer/collaboration migration 满足 `builtin-runtime-plugins`。
- cleanup gate 和分阶段删除满足 `legacy-governance-cleanup`。

## 风险与取舍

- [风险] hook contract 放进 `runtime-contract` 后语义变宽。缓解：只放纯数据/trait，不放 executor/registry。
- [风险] external hook 影响热路径。缓解：权限/规划优先 builtin in-process，external 配 timeout/failure policy。
- [风险] 迁移中出现 descriptor/executor 双事实源。缓解：每迁移一个能力就 staging 校验一一对应并删除旧路径。
- [风险] 函数注册 API 过度依赖裸 JSON payload。缓解：每个 `on_*` helper 接收对应 typed context，只有 external protocol 边界使用 JSON。
- [风险] HookContext 演变成隐式全能宿主对象。缓解：禁止直接写 durable truth；写操作只能以 typed effect 或 owner action request 表达。
- [风险] cleanup 过早删除活跃 governance 路径。缓解：每个删除任务必须先列出替代 owner、迁移调用点和通过的测试。

## 实施与迁移

1. 扩展 contract 和 descriptor。
2. 实现 plugin-host builtin/external hook binding。
3. 在 `BuiltinHookRegistry` 上补函数式注册 helper，先覆盖 planning/permission 需要的事件。
4. server 注入 dispatcher。
5. runtime 改 tool_call/tool_result/provider hook 语义。
6. host-session 接入 input/session_before_compact/model_select。
7. builtin planning/permission/composer/collaboration 迁移。
8. 清理旧 `core::hook` DTO。
9. 在替代路径验证通过后，按 cleanup gate 删除旧硬编码路径和测试 fixture。

## 验证方案

- `cargo test -p astrcode-plugin-host --lib`
- `cargo test -p astrcode-agent-runtime --lib`
- `cargo test -p astrcode-host-session --lib`
- `cargo test -p astrcode-server --lib`
- `cargo check --workspace`
- `node scripts/check-crate-boundaries.mjs`

## 未决问题

- 是否新增 `hook-contract` crate。当前建议先用 `runtime-contract`，后续需要再拆。
