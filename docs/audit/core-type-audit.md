# Core Crate 数据类型审查报告

> 审查日期：2026-04-16
> 复核日期：2026-04-16
> 审查范围：`crates/core/src/` 的主要 serde/DTO/持久化/端口数据模型，以及它们在后端消费链路中的实际用法
> 审查立场：**最佳实践、允许破坏式清理、不以兼容旧形状为前提**

---

## 审查结论

这轮审查不再把“看起来重复”直接记为问题，而是拆成两类：

1. **已证实的真实问题**
   这些问题已经能在后端真实消费链路里看到分叉、契约断裂或弱类型真相源。
2. **按最佳实践应重构的问题**
   这些问题未必已经造成运行时 bug，但在“无需兼容、追求干净 contract”的前提下，属于应主动清理的架构债。

同时，我把一批此前怀疑但目前看职责仍成立的模型降级为“已排除项”，避免把设计偏好写成缺陷。

本文件已同步到当前代码状态：

- 本轮已完成修复：`P1`、`P2`、`P3`、`P4`、`P5`、`R1`、`R3`、`R5`、`R6`、`R8`、`R9`、`R10`、`R14`、`R15`、`R16`、`R17`、`R19`、`R20`、`R21`
- 仍待后续处理：其余未标记为已修复的项

---

## 一、已修复的真实问题

### P1: `ChildSessionNotification` 同时维护两份 lifecycle 状态，且已被不同模块分别消费

**严重程度：高 | 状态：已修复**

`ChildSessionNotification` 同时包含：

```rust
child_ref: ChildAgentRef, // 内含 status
status: AgentLifecycleStatus,
```

这不是“字段长得像重复”，而是**真实双真相**：

- `session-runtime` 在 durable child node 重建时也读取 `notification.status`
  - [crates/session-runtime/src/state/child_sessions.rs](/d:/GitObjectsOwn/Astrcode/crates/session-runtime/src/state/child_sessions.rs:30)
- `server` 终端投影与 DTO 投影读取的是 `child_ref.status`
  - [crates/server/src/http/terminal_projection.rs](/d:/GitObjectsOwn/Astrcode/crates/server/src/http/terminal_projection.rs:353)
  - [crates/server/src/http/terminal_projection.rs](/d:/GitObjectsOwn/Astrcode/crates/server/src/http/terminal_projection.rs:479)

当前反序列化和构造流程没有做一致性校验，因此一旦两个字段分叉，不同后端读模型会直接得出不同结果。

**修复结果：**
- 已删除 `ChildSessionNotification.status`
- 后端读写链路统一改为消费 `child_ref.status`

---

### P2: child/session 真相被写进 `ToolExecutionResult.metadata`，但 authoritative read model 不消费它

**严重程度：高 | 状态：已修复**

`spawn` 相关链路会把 child/session 路由真相注入 `ToolExecutionResult.metadata`：

- `result_mapping` 向 metadata 注入 `agentRef` / `openSessionId`
  - [crates/adapter-tools/src/agent_tools/result_mapping.rs](/d:/GitObjectsOwn/Astrcode/crates/adapter-tools/src/agent_tools/result_mapping.rs:67)
  - [crates/adapter-tools/src/agent_tools/result_mapping.rs](/d:/GitObjectsOwn/Astrcode/crates/adapter-tools/src/agent_tools/result_mapping.rs:86)

但 conversation authoritative read model 在处理 `ToolCallResult` 时不会把这些内容提升成 typed child reference：

- [crates/session-runtime/src/query/conversation.rs](/d:/GitObjectsOwn/Astrcode/crates/session-runtime/src/query/conversation.rs:438)
- [crates/session-runtime/src/query/conversation.rs](/d:/GitObjectsOwn/Astrcode/crates/session-runtime/src/query/conversation.rs:86)

同时，前端 conversation tool block 已经只认显式 `childRef`：

- [frontend/src/lib/api/conversation.ts](/d:/GitObjectsOwn/Astrcode/frontend/src/lib/api/conversation.ts:287)
- [frontend/src/components/Chat/ToolCallBlock.tsx](/d:/GitObjectsOwn/Astrcode/frontend/src/components/Chat/ToolCallBlock.tsx:118)

这意味着后端现在维护了两套 child routing truth：

- typed notification / childRef 语义
- metadata 中的约定型 JSON 语义

而 authoritative conversation read model 只消费其中一套。结果就是：后端已经有 child/session 真相，但工具结果投影并不总能形成正式字段。

**修复结果：**
- 已为 `ToolExecutionResult` 增加 typed `child_ref`
- authoritative conversation read model 已直接消费 `child_ref`

---

### P3: `ToolExecutionResult.metadata` 已经承担正式协作 contract，不再是“附加信息”

**严重程度：高 | 状态：已修复**

`ToolExecutionResult::model_content()` 会直接反解 `metadata.agentRef`，并把其渲染为后续协作提示：

- [crates/core/src/action.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/action.rs:117)
- 对应测试固定了这种行为
  - [crates/core/src/action.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/action.rs:379)

同时，durable `StorageEventPayload::ToolResult` 也把 `metadata: Option<Value>` 持久化进事件日志：

- [crates/core/src/event/types.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/event/types.rs:112)

这说明：

- `metadata.agentRef` 不是普通展示提示
- 它已经影响模型后续的 send/observe/close 能力提示
- 它还被 durable event 结构正式保存

在最佳实践下，这种“正式协作语义藏在 `Option<Value>`”的设计应视为 contract 泄漏，而不是可接受的灵活扩展。

**修复结果：**
- `ToolExecutionResult::model_content()` 已改为读取 typed `child_ref`
- `metadata` 不再承载 child/session canonical truth

---

### P4: 旧 wire 兼容层仍在 production 类型里生效，canonical shape 不是唯一的

**严重程度：中 | 状态：已修复**

`core` 当前仍接受旧形状并在反序列化时自动补洞：

- `SubRunHandoffWire.summary -> delivery`
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:322)
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:430)
- `ChildSessionNotificationWire.summary/final_reply_excerpt -> delivery`
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:341)
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:822)

而且这不是死代码，测试明确覆盖了旧形状反序列化：

- [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1404)
- [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1436)
- [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1472)
- [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1506)

在“无需兼容”的前提下，这属于应直接删除的 contract 歧义源：写 `delivery` 才是正式形状，旧字段不应再被默许。

**修复结果：**
- 已删除旧 wire 补洞逻辑
- `SubRunHandoff` / `ChildSessionNotification` 已切到 fail-fast + canonical shape only

---

### P5: `PluginManifest` 作为外部文件契约，却没有 `deny_unknown_fields`

**严重程度：中 | 状态：已修复**

`Config` / `RuntimeConfig` / `ConfigOverlay` 等用户配置模型都显式使用了 `#[serde(deny_unknown_fields)]`：

- [crates/core/src/config.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/config.rs:64)
- [crates/core/src/config.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/config.rs:84)
- [crates/core/src/config.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/config.rs:97)

但 `PluginManifest` 作为插件作者手写的外部契约，却没有相同保护：

- [crates/core/src/plugin/manifest.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/plugin/manifest.rs:31)

这会导致 `Plugin.toml` 字段拼写错误或过期字段在解析时更容易被静默吞掉，而不是尽早暴露为配置错误。  
在“最佳实践、无需兼容”的前提下，外部清单契约应优先选择 fail-fast。

**修复结果：**
- `PluginManifest` 已增加 `#[serde(deny_unknown_fields)]`

---

## 二、按最佳实践应重构的问题

### R1: Capability canonical owner 已收口到 `core`

**优先级：高 | 状态：已修复**

此前 `core/capability.rs` 与 `protocol/capability/descriptors.rs` 各自维护一整套高度同构的
能力模型、builder 和校验逻辑，导致同一语义域存在两套真相源：

- [crates/core/src/capability.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/capability.rs:1)
- [crates/protocol/src/capability/descriptors.rs](/d:/GitObjectsOwn/Astrcode/crates/protocol/src/capability/descriptors.rs:1)

这轮已经完成收口：

- `CapabilitySpec`、`CapabilityKind`、`InvocationMode`、`PermissionSpec`、
  `SideEffect`、`Stability`、builder 与校验错误统一由 `core` 持有
- `protocol` 不再维护平行语义模型，改为显式 transport 命名
  `CapabilityWireDescriptor`，并直接复用 `core` 的 canonical 能力类型
- 插件握手协议已从 `v4` 升级到 `v5`，wire shape 删除 `streaming: bool`，
  改为直接传输 `invocation_mode`
- `plugin` 侧 `capability_mapping` 已收缩为边界 adapter，
  `sdk` 面向插件作者公开的能力模型也已切到 `CapabilitySpec`

当前结论：

- `core` 是 capability 语义的唯一 owner
- `protocol` 只保留 transport/read-model 上下文与显式 wire 命名
- `sdk` / `plugin` 已不再围绕旧的 protocol capability DTO 术语建模

---

### R2: Protocol DTO 层仍大规模镜像 Core 类型

**优先级：高 | 状态：已按分类治理**

`protocol/http/agent.rs`、`protocol/http/event.rs` 仍然镜像了大量 `core` 的协作和事件模型：

- [crates/protocol/src/http/agent.rs](/d:/GitObjectsOwn/Astrcode/crates/protocol/src/http/agent.rs:1)
- [crates/protocol/src/http/event.rs](/d:/GitObjectsOwn/Astrcode/crates/protocol/src/http/event.rs:1)

这不是本轮最硬的 bug，但在“最佳实践、无需兼容”的立场下，这种镜像层应尽量缩减到真正的传输差异，而不是复制核心类型。

**当前结论：**
- `A 类已收口`
  `protocol/http/event.rs` 中与 `core` serde 形状完全一致的真镜像 DTO 已改为直接 re-export：
  `PhaseDto`、`ToolOutputStreamDto`、`ForkModeDto`、`SubRunStorageModeDto`、
  `SubRunFailureCodeDto`、`ArtifactRefDto`、`ParentDelivery*Dto`、`SubRunHandoffDto`
- `B 类边界 DTO 继续保留显式协议投影`
  `SubRunResultDto`、`ResolvedSubagentContextOverridesDto`、`RuntimeCapabilityDto`
  仍承担 wire/read-model 语义，不再为了“数量收口”强行并回 core
- `C 类 transport/read-model DTO 不再记为镜像坏味道`
  `AgentExecute*Dto`、`ToolExecute*Dto`、`Conversation*Dto`、`Terminal*Dto`、
  `RuntimeStatusDto`、`Composer*Dto` 等本来就是协议层正式模型，应刻意保留
- `server/http/mapper.rs` 中对应的纯机械 A 类转换函数已删除，映射层现在只保留真正的协议投影与请求解析

---

### R3: `SubRunResult` 与 `CollaborationResult` 的建模风格不一致

**优先级：高 | 状态：已修复**

当前两套“协作结果”模型风格完全不同：

- `SubRunResult` 依赖字段组合表达状态
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:462)
- `CollaborationResult` 使用 `kind + 多个 Option` 的大 struct
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:927)

这项已经在本轮收口完成：

- `SubRunResult` 已改为判别联合，区分 `Running / Completed / Failed`
- `CollaborationResult` 已改为按动作分支的 variant，移除了 `kind + Option` 矩阵
- 已进一步引入 `SubRunStatus` 作为正式状态投影，外围不再依赖 `lifecycle + last_turn_outcome` 组合推导
- 已移除 `CollaborationResult.accepted()` / `action_kind()` 这类残留的伪组合 helper
- `application` / `adapter-tools` / `server` 的消费侧已同步切到 helper/variant 访问

---

### R4: `HookCompactionReason` 与 `CompactTrigger` 分裂表达同一语义域

**优先级：中 | 状态：已修复**

- [crates/core/src/hook.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/hook.rs:43)
- [crates/core/src/event/types.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/event/types.rs:57)

两者都在表达“上下文压缩触发原因”，只是一个多了 `Reactive`。  
这更像命名与分层漂移，不是运行时 bug；但在最佳实践下，应合并为统一枚举并在需要的层做受限投影。

**修复结果：**
- 已删除 `HookCompactionReason`
- hook 压缩上下文统一复用 `CompactTrigger`

---

### R5: child identity 三元组在多个结构里散落复制

**优先级：中 | 状态：已修复**

以下结构都重复维护 `agent_id / session_id / sub_run_id` 及其 parent 变体：

- `SubRunHandle`
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:629)
- `ChildAgentRef`
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:696)
- `ChildSessionNode`
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:730)
- `AgentCollaborationFact`
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1008)
- `AgentEventContext`
  - [crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1082)

这项我不再称为“已证实 bug”，但它确实导致 projection/helper 需要反复逐字段拼装。  
在最佳实践下，应抽出更明确的 lineage/identity 子结构。

**修复结果：**
- 已抽出 `ChildExecutionIdentity` 与 `ParentExecutionRef`
- `ChildAgentRef` / `ChildSessionNode` 已改为组合这两个子结构，而不再平铺复制 child identity
- `application` / `session-runtime` / `server` 已统一改用 accessor/helper 消费正式 identity

---

### R6: `ChildAgentRef` / `SubRunHandle` / `ChildSessionNode` 的 projection 边界没有被类型系统彻底收住

**优先级：中 | 状态：已修复**

之前虽然 `ChildExecutionIdentity` / `ParentExecutionRef` 已经抽出，但 live/durable/external 三层 projection 仍然散落在外围 helper：

- `SubRunHandle` 是 live runtime owner
- `ChildSessionNode` 是 durable lineage node
- `ChildAgentRef` 是 external projection

而 `application` / `session-runtime` 里仍有多处手工拼装 `ChildAgentRef` 和 `ChildSessionNode`，意味着边界依然主要靠调用约定维持。

**修复结果：**
- 已在 `SubRunHandle` 下沉 `child_identity()`、`parent_ref()`、`open_session_id()`、`child_ref()`、`child_ref_with_status()`
- 已在 `ChildAgentRef` 下沉 `to_child_session_node(...)`
- `application` / `session-runtime` 主链路已改为调用这些 canonical helper，而不是手工逐字段拼 projection

---

### R7: `ResolvedConfig` 系列不是错误设计，但 resolve 样板仍然过重

**优先级：低**

`Config` / `RuntimeConfig` / `AgentConfig` 与对应 `Resolved*` 形态的分层本身成立；
我不再把它记成缺陷。

但从最佳实践看，`resolve_runtime_config` 一类函数仍有较重的逐字段样板，可考虑后续用 derive/macro 收敛：

- [crates/core/src/config.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/config.rs:97)
- [crates/core/src/config.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/config.rs:203)

---

### R8: 强类型 ID（SessionId/TurnId/AgentId）定义后未被充分使用

**优先级：中 | 状态：已修复（主链路）**

`ids.rs` 定义了 `SessionId`、`TurnId`、`AgentId`、`CapabilityName` 四个强类型 newtype，
但 core 内部大量结构体仍使用裸 `String`：

- `session_id: String` 出现 20+ 处（`SubRunHandle`、`ChildAgentRef`、`ChildSessionNode`、`AgentState` 等）
- `turn_id: String` 出现 20+ 处（`AgentEvent` 各变体、`SubRunHandle`、`ToolHookContext` 等）
- `agent_id: String` 出现 19 处

同时存在不一致：
- `EventStore` trait（ports.rs:39）的 `try_acquire_turn` 用 `turn_id: &str`
- 同文件 `PromptFactsRequest`（ports.rs:230）用 `turn_id: Option<TurnId>`
- `runtime/traits.rs` 的 `ExecutionAccepted` 正确使用了 `SessionId`/`TurnId`/`AgentId`

这项在本轮已经完成主链路收口：

- `SubRunHandle`、`ChildAgentRef`、`ChildSessionNode`、`AgentCollaborationFact`、`AgentEventContext` 已迁移到强类型 ID
- `application` / `session-runtime` / `kernel` / `server` 的边界投影已改为显式 `to_string()` 或 `into()`

剩余若还有零散裸 `String`，应视为后续扫尾，而不是主链路仍未落地。

---

### R9: `SubRunExecutionOutcome` 是 `AgentTurnOutcome` 的近完全重复

**优先级：中 | 状态：已修复**

`observability.rs` 中的 `SubRunExecutionOutcome { Completed, Failed, Aborted, TokenExceeded }`
与 `AgentTurnOutcome { Completed, Failed, Cancelled, TokenExceeded }` 语义完全相同，
仅 `Cancelled` → `Aborted` 命名差异。

`application/terminal.rs:433-439` 原先有手工转换函数做一对一映射。
本轮已删除 `SubRunExecutionOutcome`，观测接口直接复用 `AgentTurnOutcome`。

---

### R10: `PromptSkillSummary` 与 `PromptAgentProfileSummary` 字段完全重复

**优先级：低 | 状态：已修复**

```rust
pub struct PromptSkillSummary { pub id: String, pub description: String }
pub struct PromptAgentProfileSummary { pub id: String, pub description: String }
```

两者字段完全相同，已合并为统一的 `PromptEntrySummary`。


### R12: `ForkMode` 在 core 层无任何业务消费

**优先级：低 | 状态：已修复**

`ForkMode { FullHistory, LastNTurns(usize) }` 在 `agent/mod.rs:76` 定义，
通过 `SubagentContextOverrides`/`ResolvedSubagentContextOverrides` 传递，
但 core 内部没有任何代码 match 其变体做分支决策。

它原本只被 `server/http/mapper.rs` 做 Dto 映射，在 core 层是透传数据，不参与任何核心逻辑。

**修复结果：**
- `session-runtime` 已按 `ForkMode` 实际裁剪子执行继承的父对话 tail
- `None`/`FullHistory` 与 `LastNTurns(n)` 现在有真实业务分支，不再是死传参

---

### R13: `InvocationMode::Streaming` 在 core 层无分支消费

**优先级：低**

`InvocationMode` 有 `Unary` 和 `Streaming` 两个变体，但 core 内部没有任何代码
match `invocation_mode` 做分支判断。它只被设置、传递、存储。
`Streaming` 变体的实际消费全部在 plugin/adapter 层。

如果 core 不需要感知 streaming 语义，可以考虑将此概念下沉到实际消费的层。

---

### R14: validate 校验模式在多处重复——`agent_id.trim().is_empty()` 出现 5 次

**优先级：低 | 状态：已修复**

以下结构体有完全相同的 `agent_id.trim().is_empty()` 校验：

- `SendToChildParams`（agent/mod.rs）
- `CloseAgentParams`（agent/mod.rs）
- `SendParams`（mailbox.rs）
- `ObserveParams`（mailbox.rs）
- `CloseParams`（mailbox.rs）

`message.trim().is_empty()` 也在两处重复。

**修复结果：**
- 已在 `agent/mod.rs` 提取 `require_non_empty_trimmed()` 与 `require_not_whitespace_only()`
- `agent/mod.rs` 与 `agent/mailbox.rs` 的相关 `validate()` 已统一复用这些 helper

---

### R15: `ExecutionOwner.root_turn_id` 用裸 `String` 而非 `TurnId`

**优先级：低 | 状态：已修复**

`ExecutionOwner` 正确使用了 `SessionId` 但 `root_turn_id` 仍是 `String`：
`tool.rs:41`：`pub root_turn_id: String`

该字段已迁移为 `TurnId`。

---

### R16: `CurrentModelSelection` 与 `ModelOption` 字段完全相同

**优先级：低 | 状态：已修复**

[crates/core/src/config.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/config.rs:376) 中两个结构体字段完全相同：

```rust
pub struct CurrentModelSelection {  // L376
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}

pub struct ModelOption {            // L384
    pub profile_name: String,
    pub model: String,
    pub provider_kind: String,
}
```

两者已合并为 `ModelSelection`，并通过 type alias 保留语境名。

---

### R17: `RuntimeCoordinator` 使用 `expect` 处理 lock poisoning，未复用 `support.rs` 提供的恢复工具

**优先级：低 | 状态：已修复**

[crates/core/src/runtime/coordinator.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/runtime/coordinator.rs:74) 中有 6+ 处 `.expect("... lock poisoned")`：

```rust
self.managed_components.write()
    .expect("runtime coordinator managed components lock poisoned") = managed_components;
```

但同 crate 的 [crates/core/src/support.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/support.rs) 已经提供了 `with_write_lock_recovery` 等优雅恢复工具。

lock poisoning 在生产环境中虽然罕见，但一旦发生，`expect` 会直接 panic 并带走整个进程。
应统一使用 `support.rs` 的恢复工具，或至少在 coordinator 中应用一致的 lock 处理策略。

**修复结果：**
- `RuntimeCoordinator` 已统一改用 `support::with_read_lock_recovery()` / `with_write_lock_recovery()`
- `with_managed_components()`、`capabilities()`、`managed_components()`、`replace_runtime_surface()`、`shutdown()` 均已移除生产代码中的 lock poisoning `expect`

---

### R19: `AgentCollaborationFact` 中 child identity 仍以平铺字段重复表达

**优先级：中 | 状态：已修复**

[crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:1072) 原先在 `AgentCollaborationFact` 中同时维护：

```rust
pub child_agent_id: Option<AgentId>
pub child_session_id: Option<SessionId>
pub child_sub_run_id: Option<SubRunId>
```

虽然主链路强类型 ID 已经到位，但这里仍然保留一套平铺的 child identity 三元组，
与 `ChildExecutionIdentity` / `ChildAgentRef` 的正式语义重复。  
这会导致：

- `application` 构造协作事实时继续手工拆散 child identity
- `observability` 等消费侧继续依赖 `fact.child_agent_id`
- R5 已完成的 identity 收口在协作事实层被重新打散

**修复结果：**
- `AgentCollaborationFact` 已改为 `child_identity: Option<ChildExecutionIdentity>`
- 新增 `child_agent_id()` / `child_session_id()` / `child_sub_run_id()` accessor，消费侧统一通过 helper 读取
- `application` / `session-runtime` / `observability` 与测试夹具已同步迁移

---

### R20: `SendAgentParams` 使用 `#[serde(untagged)]` 反序列化，缺乏判别依据

**优先级：中 | 状态：已修复**

[crates/core/src/agent/mod.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs:887)：

```rust
#[serde(untagged)]
pub enum SendAgentParams {
    ToChild(SendToChildParams),
    ToParent(SendToParentParams),
}
```

`untagged` 反序列化完全依赖字段匹配来区分变体，而 `SendToParentParams` 使用 `#[serde(flatten)]`
包裹 `ParentDeliveryPayload`（自身是 `tag + content` 形式），这导致：
- 反序列化顺序敏感：如果 `ParentDeliveryPayload` 的 `kind` 字段恰好出现在输入 JSON 中，
  serde 会优先尝试 `ToParent` 变体
- 错误信息不友好：`untagged` 失败时所有变体的错误信息被合并，调试困难
- 与 codebase 中其他 tagged enum（如 `ParentDeliveryPayload`）的风格不一致

**修复结果：**
- 已改为显式 `direction` 判别字段
- tool schema、说明文案和测试已同步为新形状

---

### R21: `CapabilityExecutionResult` 与 `ToolExecutionResult` 近完全重复

**优先级：中 | 状态：已修复**

[crates/core/src/registry/router.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/registry/router.rs:77) 的 `CapabilityExecutionResult` 与 [crates/core/src/action.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/action.rs:61) 的 `ToolExecutionResult` 字段高度重叠：

| CapabilityExecutionResult | ToolExecutionResult | 差异 |
|---|---|---|
| `capability_name: String` | `tool_name: String` | 命名不同 |
| `success: bool` | `ok: bool` | 命名不同 |
| `output: Value` | `output: String` | 类型不同 |
| `error: Option<String>` | `error: Option<String>` | 相同 |
| `metadata: Option<Value>` | `metadata: Option<Value>` | 相同 |
| `duration_ms: u64` | `duration_ms: u64` | 相同 |
| `truncated: bool` | `truncated: bool` | 相同 |
| — | `tool_call_id: String` | ToolExecutionResult 独有 |

而且 `CapabilityExecutionResult::into_tool_execution_result()` 原先直接做逐字段映射。
本轮已提取共享的 `ExecutionResultCommon`，统一承载：

- `error`
- `metadata`
- `duration_ms`
- `truncated`

两种结果类型保留各自的输出/命名差异，但共享公共执行结果内核，并通过
`common()` / `with_common()` 显式完成跨类型映射，避免继续散落手写四字段复制。

---

## 三、已排除项

以下模型在本轮复核后，**暂不记为问题**：

- `PromptFacts` / `PromptBuildRequest` / `PromptBuildOutput`
  - [crates/core/src/ports.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/ports.rs:239)
  - [crates/core/src/ports.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/ports.rs:279)
  - [crates/core/src/ports.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/ports.rs:315)
  - 这里的 `metadata: Value` 更像开放扩展面，而不是已经泄漏成正式真相源
- `ResourceRequestContext` / `ResourceReadResult`
  - [crates/core/src/ports.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/ports.rs:334)
  - [crates/core/src/ports.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/ports.rs:348)
  - 仍属于适配层互操作上下文，暂未看到 contract 漂移
- `CapabilityCall` / `ApprovalRequest` / `PolicyContext`
  - [crates/core/src/policy/engine.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/policy/engine.rs:86)
  - [crates/core/src/policy/engine.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/policy/engine.rs:110)
  - [crates/core/src/policy/engine.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/policy/engine.rs:186)
  - 虽然同样使用 `metadata: Value`，但目前没有证据表明它已经变成正式业务真相源
- `AgentMailboxEnvelope`
  - [crates/core/src/agent/mailbox.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/agent/mailbox.rs:40)
  - `sender_lifecycle_status` / `sender_last_turn_outcome` / `sender_open_session_id` 是明确的 enqueue-time snapshot，语义自洽
- `event/domain.rs` 与 `event/types.rs`
  - [crates/core/src/event/domain.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/event/domain.rs:40)
  - [crates/core/src/event/types.rs](/d:/GitObjectsOwn/Astrcode/crates/core/src/event/types.rs:69)
  - “展示事件”和“持久化事件”分层仍然成立，不构成简单重复

---

## 四、推荐重构顺序

| 优先级 | 项目 | 原因 | 建议动作 |
|---|---|---|---|
| **Done** | P1 Notification 双状态真相 | 已修复 | 已删除外层 `status`，统一收敛到 `child_ref.status` |
| **Done** | P2 + P3 Tool metadata 承载协作真相 | 已修复 | 已引入 typed `child_ref` 并移除 metadata 真相职责 |
| **Done** | P4 旧 wire 兼容层 | 已修复 | 已删除旧字段和旧 wire 补洞函数 |
| **P1** | R1 Core/Protocol capability canonical owner | 长期冗余最高 | 收口 capability 描述体系的 canonical owner |
| **Done** | R2 event 真镜像 DTO 收口 | 已修复 | `protocol/http/event.rs` A 类 DTO 已改为 core re-export，mapper 机械转换已删除 |
| **P2** | R3 协作结果建模统一 | 降低后端消费复杂度 | 将 `SubRunResult` / `CollaborationResult` 改成统一 variant 模型 |
| **Done** | R5 child identity 收口 | 已修复 | 已引入 `ChildExecutionIdentity` / `ParentExecutionRef` 并统一消费链路 |
| **P2** | R6 projection 边界继续收紧 | 降低 projection/clone 样板 | 继续限制 `ChildAgentRef` / `SubRunHandle` / `ChildSessionNode` 之间的投影入口 |
| **P2** | R8 强类型 ID 未充分使用 | 提高类型安全 | 逐步将 key DTO 的 ID 字段迁移为强类型 |
| **Done** | P5 Plugin manifest fail-fast | 已修复 | 已为 `PluginManifest` 增加 `deny_unknown_fields` |
| **P3** | R7 config resolve 样板治理 | 清理样板 | 后续再处理 config resolve 样板 |
| **Done** | R9 + R10 次级类型重复 | 已修复 | 已复用 `AgentTurnOutcome`；已合并 Prompt summary 类型 |
| **P3** | R11 SubRunStorageMode 单变体枚举 | 简化无分支抽象 | 退化为 marker struct 或 ZST |
| **Done** | R12 ForkMode 无业务消费 | 已修复 | session-runtime 已真实消费 ForkMode |
| **P3** | R13 Streaming 变体在 core 无消费 | 对齐抽象层级 | 下沉到 plugin/adapter 层 |
| **Done** | R14 validate 校验模板重复 | 已修复 | 已提取通用 trim 校验 helper 并统一复用 |
| **Done** | R15 ExecutionOwner 类型不一致 | 已修复 | `root_turn_id` 已改用 `TurnId` |
| **Done** | R16 CurrentModelSelection/ModelOption 重复 | 已修复 | 已合并为统一 `ModelSelection` 类型 |
| **Done** | R17 RuntimeCoordinator lock 处理 | 已修复 | `RuntimeCoordinator` 已统一复用 `support.rs` 锁恢复工具 |
| **Done** | R19 AgentCollaborationFact identity 重复 | 已修复 | 已收口为 `child_identity: Option<ChildExecutionIdentity>` 并迁移消费侧 |
| **Done** | R20 SendAgentParams untagged 反序列化 | 已修复 | 已改为显式 tagged `direction` |
| **Done** | R21 CapabilityExecutionResult/ToolExecutionResult 重复 | 已修复 | 已提取共享 `ExecutionResultCommon` 并收口关键映射链路 |

---

## 五、总结统计

| 类别 | 当前结论 |
|---|---|
| 已修复的真实问题 | `P1` ~ `P5` 已全部收口 |
| 已修复的最佳实践重构 | `R2`、`R3`、`R4`、`R5`、`R6`、`R8`、`R9`、`R10`、`R12`、`R14`、`R15`、`R16`、`R17`、`R19`、`R20`、`R21` 已完成 |
| 按最佳实践仍待重构的问题 | 主要剩 `R1`、`R7`、`R11`、`R13` |
| 已排除项 | 仍以第三节列出的 5 类为准 |
| 重点风险域 | 协作 / 子会话 / 工具结果 / 外部契约 |

当前最硬的问题不在 `core` 的所有 DTO 上平均分布，而是明显集中在：

1. child/session 协作链路的 canonical truth 管理
2. `metadata: Value` 被误用为正式 contract
3. old wire shape 与 new shape 并存导致的 canonical 不唯一
