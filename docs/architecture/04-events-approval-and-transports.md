# Events, Approval, and Transports

## Goal

这份文档专门回答三个容易混淆的问题：

1. 异步观测和同步决策如何分离
2. 审批请求应该通过什么机制挂起和恢复
3. transport 层应该如何订阅 runtime，而不是反向主导 runtime

## 1. Event Is Observation, Not Control

`Event` 的职责是：

- 广播运行时事实
- 驱动 UI
- 支持 telemetry / audit / debug
- 支持 CLI / ACP / SSE 等外部观察者

`Event` 不负责：

- 审批最终决定
- 权限放行
- compaction 触发策略
- provider request 改写

这些能力应留在 Policy 或专门 runtime service 里。

## 2. Approval Needs a Broker, Not Just a Bus

如果 policy 返回 `Ask`，runtime 必须挂起并等待一个明确回复。  
这不是纯广播模式能天然解决的问题。

因此建议明确引入：

```rust
trait ApprovalBroker: Send + Sync {
    async fn request(
        &self,
        req: ApprovalRequest,
        cancel: CancelToken,
    ) -> Result<ApprovalResolution>;
}
```

执行顺序应当是：

1. `PolicyEngine` 返回 `Ask`
2. runtime 调用 `ApprovalBroker::request()`
3. `ApprovalBroker` 负责等待用户或外部客户端回复
4. runtime 根据结果继续 Allow / Deny 分支
5. 同时把 `ApprovalRequested` / `ApprovalResolved` 发到 `EventBus`

这样做的好处：

- 审批不会依赖某个具体 transport
- CLI、Web、Tauri、ACP 都能走同一审批模型
- 审批状态不会被混成“只是一个 UI 事件”
- broker 可以显式感知 turn cancellation，而不是在挂起审批时泄漏僵尸等待

当前实现中，runtime 默认提供一个 `DefaultApprovalBroker`：  
它会根据 `ApprovalRequest.default` 立即给出 allow / deny 结果。这样在真正的 Web/CLI 审批 transport 到位之前，`Ask` 也不会把 turn 卡死。

## 3. EventBus vs EventLog

建议明确区分：

### EventBus

面向实时观察者。

典型实现：

- `tokio::broadcast`
- fan-out
- best-effort
- 适合 SSE、CLI、UI、telemetry

### EventLog

面向持久化和回放。

典型实现：

- append-only session event store
- 支持 replay
- 支持 cursor / sequence id
- 支持恢复和调试

AstrCode 当前已经有 `StorageEvent` 与 replay 体系。  
未来不建议把所有瞬时 `AgentEvent` 都直接等同为持久化事件。

推荐关系是：

```text
AgentLoop
  ├─ emits AgentEvent to EventBus
  └─ appends durable StorageEvent to EventLog
```

某些事件可以双写，某些只需要其中一种。

## 4. Transports Are Adapters

transport 层只做协议暴露和客户端适配，不做 agent 语义定义。

### HTTP/SSE

职责：

- 接收命令
- 订阅 EventBus
- 从 EventLog 提供 replay / cursor
- 输出 HTTP DTO

当前代码锚点：

- `crates/server/src/main.rs`
- `crates/server/src/routes/*`
- `frontend/src/hooks/useAgent.ts`

### Tauri/Web

职责：

- UI 展示
- 调用 HTTP API
- 展示审批、工具输出和流式事件

当前代码锚点：

- `src-tauri/src/main.rs`
- `frontend/src/hooks/useAgent.ts`

### CLI / ACP

职责：

- 作为 runtime 的外部观察者和控制器
- 走同一套 event / approval / command 边界

这类接入层应在设计阶段预留，不必第一阶段做满。

## 5. Recommended Event Flow

```text
Policy allow/deny/ask
    ↓
AgentLoop executes or suspends
    ↓
ApprovalBroker resolves pending ask
    ↓
EventBus broadcasts runtime facts
    ↓
HTTP/SSE / CLI / ACP / Tauri/Web subscribe and render
```

这个顺序很关键：

- 先决策
- 再执行
- 再观测

不要反过来让 transport 或 UI 决定 agent 内核的行为。

## 6. Transport-Safe Principles

为避免后续 UI 或协议接入污染 core，建议坚持以下规则：

- `AgentLoop` 不直接依赖 HTTP/SSE
- `ApprovalBroker` 不直接依赖 Web/Tauri 组件
- `EventBus` 只传播领域事件，不传播 UI 组件状态
- transport DTO 不直接等于 core 类型
- ACP / CLI / SSE 共享 runtime 事件，不共享彼此的 transport shape

这和现有 V4 protocol ADR 方向是一致的：传输层只做 raw message transport，状态机与业务语义不应下沉到 transport。

## Current Status

截至当前实现：

- Phase 3 的 `PolicyEngine` 三态与 `ApprovalBroker` 已落地进 `AgentLoop`
- `Allow / Deny / Ask` 已经真正影响 tool-call 执行路径
- runtime 默认 broker 已存在，且支持 cancel-aware request
- Phase 4 的 runtime observation bus 仍未落地，审批状态目前不会单独作为 durable session event 存储
