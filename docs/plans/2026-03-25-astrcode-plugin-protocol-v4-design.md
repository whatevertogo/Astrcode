# AstrCode Plugin Protocol V4 设计稿

> 日期：2026-03-25  
> 状态：Draft  
> 目标：定义 AstrCode 核心进程与插件 / worker / 可替换 runtime 之间的协议骨架，保留 v4 分层思想，并支持 coding-first + general-agent-ready 的长期演进。

---

## 1. 设计结论

AstrCode Protocol V4 采用：

- **v4 风格的四层结构**
  - Transport
  - Protocol Peer
  - Capability Router / Handler Dispatcher
  - Business Logic
- **五类基础消息**
  - `InitializeMessage`
  - `InvokeMessage`
  - `ResultMessage`
  - `EventMessage`
  - `CancelMessage`
- **双层语义模型**
  - `Core Protocol`
  - `Execution Profile`

其中：

- Core Protocol 保持尽量通用
- `coding` 作为首个官方 profile
- future `general-agent` / `workflow` 等 profile 通过扩展进入

---

## 2. 为什么不用“纯编码专用协议”

如果把协议直接设计成“文件编辑器专用线协议”，短期会更快，但长期会有三个问题：

1. 插件协议会和当前产品 UI / 当前 runtime 过度耦合
2. 想支持更通用 agent 时，要重写 descriptor / invoke / streaming 模型
3. Rust SDK 与未来多语言 SDK 很难共用

因此更合理的方向是：

- 把 **消息骨架** 做通用
- 把 **能力语义** 做专业
- 把 **coding 特征** 放进 profile / context / descriptor 扩展

---

## 3. 协议分层

```text
┌─────────────────────────────────────────┐
│            Business Logic               │
│ tool / agent / context / memory / hook │
├─────────────────────────────────────────┤
│ Capability Router / Handler Dispatcher │
├─────────────────────────────────────────┤
│             Protocol Peer               │
├─────────────────────────────────────────┤
│               Transport                 │
└─────────────────────────────────────────┘
```

### 3.1 Transport

只负责字符串收发。

- `stdio`
- `websocket`
- future transports

Transport 不处理：

- 协议解析
- request/response 关联
- 权限
- 业务路由

### 3.2 Protocol Peer

负责：

- 握手
- 协议版本协商
- 请求 ID 关联
- 非流式 / 流式调用
- 取消传播
- 连接异常统一收敛

### 3.3 Capability Router / Handler Dispatcher

负责：

- capability 注册
- handler 注册
- 权限预检查
- profile 筛选
- 本地路由

### 3.4 Business Logic

真正的 tool / agent / context / memory / hook 实现。

---

## 4. 消息模型

## 4.1 InitializeMessage

用途：建立连接、交换 peer 身份、声明 capabilities / handlers、协商版本。

建议字段：

```rust
pub struct InitializeMessage {
    pub r#type: "initialize",
    pub id: String,
    pub protocol_version: String,
    pub supported_protocol_versions: Vec<String>,
    pub peer: PeerDescriptor,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub handlers: Vec<HandlerDescriptor>,
    pub profiles: Vec<ProfileDescriptor>,
    pub metadata: serde_json::Value,
}
```

说明：

- 初始化结果不再独立发第六种消息
- 初始化响应通过 `ResultMessage { kind: "initialize" }` 返回

## 4.2 InvokeMessage

用途：发起一次能力调用。

```rust
pub struct InvokeMessage {
    pub r#type: "invoke",
    pub id: String,
    pub capability: String,
    pub input: serde_json::Value,
    pub context: InvocationContext,
    pub stream: bool,
}
```

说明：

- `context` 是长期关键字段
- coding 语义不直接进入顶层，而进入 `context.profile_context`

## 4.3 ResultMessage

用途：返回非流式调用结果，或初始化结果。

```rust
pub struct ResultMessage {
    pub r#type: "result",
    pub id: String,
    pub kind: Option<String>,
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<ErrorPayload>,
    pub metadata: serde_json::Value,
}
```

建议：

- `kind = "initialize"` 表示握手结果
- 其他普通调用 `kind = None` 即可

## 4.4 EventMessage

用途：流式事件。

```rust
pub struct EventMessage {
    pub r#type: "event",
    pub id: String,
    pub phase: EventPhase,
    pub event: String,
    pub payload: serde_json::Value,
    pub seq: u64,
    pub error: Option<ErrorPayload>,
}
```

其中：

```rust
pub enum EventPhase {
    Started,
    Delta,
    Completed,
    Failed,
}
```

说明：

- 生命周期简单固定
- 具体业务语义通过 `event` 表达，例如：
  - `message.delta`
  - `reasoning.delta`
  - `tool.call.started`
  - `artifact.patch`
  - `diagnostic`

## 4.5 CancelMessage

用途：取消调用。

```rust
pub struct CancelMessage {
    pub r#type: "cancel",
    pub id: String,
    pub reason: Option<String>,
}
```

要求：

- 支持早到取消
- 支持已开始任务的协作式取消

---

## 5. Descriptor 模型

## 5.1 PeerDescriptor

```rust
pub struct PeerDescriptor {
    pub id: String,
    pub name: String,
    pub role: PeerRole,
    pub version: String,
    pub supported_profiles: Vec<String>,
    pub metadata: serde_json::Value,
}
```

推荐 role：

- `core`
- `plugin`
- `runtime`
- `worker`
- `supervisor`

## 5.2 CapabilityDescriptor

```rust
pub struct CapabilityDescriptor {
    pub name: String,
    pub kind: CapabilityKind,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub streaming: bool,
    pub profiles: Vec<String>,
    pub tags: Vec<String>,
    pub permissions: Vec<PermissionHint>,
    pub side_effect: SideEffectLevel,
    pub stability: StabilityLevel,
}
```

推荐 kind：

- `tool`
- `agent`
- `context_provider`
- `memory_provider`
- `policy_hook`
- `renderer`
- `resource`

## 5.3 HandlerDescriptor

用于事件订阅 / hook / command trigger。

```rust
pub struct HandlerDescriptor {
    pub id: String,
    pub trigger: TriggerDescriptor,
    pub input_schema: serde_json::Value,
    pub profiles: Vec<String>,
    pub filters: Vec<FilterDescriptor>,
    pub permissions: Vec<PermissionHint>,
}
```

---

## 6. InvocationContext 与 Coding Profile

## 6.1 Base Context

```rust
pub struct InvocationContext {
    pub request_id: String,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub caller: Option<CallerRef>,
    pub workspace: Option<WorkspaceRef>,
    pub deadline_ms: Option<u64>,
    pub budget: Option<BudgetHint>,
    pub profile: String,
    pub profile_context: serde_json::Value,
    pub metadata: serde_json::Value,
}
```

## 6.2 Coding Profile Context

`profile = "coding"` 时，`profile_context` 推荐包含：

- `working_dir`
- `repo_root`
- `open_files`
- `active_file`
- `selection`
- `branch`
- `approval_mode`
- `output_mode`
- `language_hints`

原则：

- Base Context 保持通用
- coding 细节全部进 profile context

---

## 7. StreamExecution 设计

流式调用统一表达为：

1. `Event(started)`
2. `Event(delta)*`
3. `Event(completed)` 或 `Event(failed)`

不建议为每种流式内容增加新的消息类型。

建议通过 `event` 名称分类：

- `message.delta`
- `reasoning.delta`
- `progress.step`
- `tool.call.started`
- `tool.call.completed`
- `artifact.patch`
- `artifact.file`
- `diagnostic`
- `log`

这样：

- 生命周期稳定
- 领域语义丰富
- 对通用 agent 仍然友好

---

## 8. 权限与副作用

CapabilityDescriptor 必须能声明权限与副作用。

推荐权限：

- `filesystem.read`
- `filesystem.write`
- `process.exec`
- `network.http`
- `repo.write`
- `secrets.read`
- `model.invoke`

推荐副作用：

- `none`
- `local`
- `workspace`
- `external`

注意：

- 协议只声明请求和能力需求
- 最终授权由 Core Policy 决定

---

## 9. 对当前 AstrCode Rust 仓库的落地建议

## 9.1 `crates/protocol`

建议补齐：

- `plugin/messages.rs`
- `plugin/descriptors.rs`
- `plugin/error.rs`
- `plugin/profile.rs`

当前的 `InitializeResult` 独立消息长期建议收敛为 `Result(kind="initialize")`。

## 9.2 `crates/plugin`

建议逐步补齐：

- `peer.rs`
- `capability_router.rs`
- `handler_dispatcher.rs`
- `_streaming.rs`
- `supervisor.rs`
- `worker.rs`

当前仅有 loader / process / executor / handshake / transport，尚未形成完整 v4 运行时骨架。

## 9.3 `crates/sdk`

建议调整为：

- 基础 SDK：descriptor、context、stream、error、registration
- coding profile SDK：workspace / patch / diagnostic / approval helper

不要把 coding helper 直接塞进所有插件都必须依赖的最底层 trait。

---

## 10. 最终方向

AstrCode Protocol V4 的正确方向不是：

- 把协议做成“只会读写文件”的专用协议

而是：

- 保留 v4 的 peer / router / stream 思想
- 保留五类基础消息
- 让 coding 成为第一优先 profile
- 让 future general-agent 扩展仍然有空间

这是最适合 AstrCode 长期变成“面向编码场景的智能体平台”的路线。
