# ADR-0005: Split Policy Decision Plane from Event Observation Plane

- Status: Accepted
- Date: 2026-03-30
- Amended: 2026-04-03

## Context

AstrCode 后续必须同时支持两类能力：
- 能改变执行结果的同步决策，例如权限、审批、上下文压力处理
- 只观察运行时事实的异步订阅，例如 UI、SSE、telemetry、audit

如果继续把两者混在一个泛化 hook / wire 机制里，会出现以下问题：
- `Allow / Deny / Ask` 与纯观测事件耦合
- transport 或 UI 容易被误当成核心决策者
- `Ask` 需要挂起与恢复，天然不同于广播观察
- durable event log 与瞬时 runtime event 难以分层

## Decision

冻结 AstrCode 的控制面与观测面为两条不同契约：

### 1. Policy 是唯一同步决策面

Policy contract 拥有改变执行路径的权力，包括：
- 允许 (Allow)
- 拒绝 (Deny)
- 请求审批 (Ask)
- 改写输入
- 决定 context pressure 下的处理策略

核心契约位于 `crates/core/src/policy/`：
```rust
// crates/core/src/policy/mod.rs
trait PolicyEngine {
    fn check_model_request(&self, input: &ModelRequest, ctx: &PolicyContext)
        -> PolicyVerdict<ModelRequest>;
    fn check_capability_call(&self, call: &CapabilityCall, ctx: &PolicyContext)
        -> PolicyVerdict<CapabilityCall>;
    fn decide_context_strategy(&self, input: &ContextPressureInput)
        -> PolicyVerdict<ContextStrategyDecision>;
}

enum PolicyVerdict<T> {
    Allow(T),
    Deny { reason: String },
    Ask(ApprovalPending<T>),
}
```

三个决策点：
- `check_model_request` — LLM 调用前的策略检查 (`crates/runtime/src/agent_loop/turn_runner.rs`)
- `check_capability_call` — 工具调用前的三态分支 (`crates/runtime/src/agent_loop/tool_cycle.rs`)
- `decide_context_strategy` — 上下文压力决策 (待接入 token budgeting / compaction)

默认实现：`AllowAllPolicyEngine` (`crates/core/src/policy/engine.rs`)

### 2. Event 是唯一异步观测面

Event contract 只表达运行时事实，不拥有改变执行结果的权力。

两类事件通过 `EventTranslator` 投影，不强制等同：

| 类型 | 源码路径 | 用途 | 消费者 |
|------|---------|------|--------|
| `AgentEvent` | `crates/core/src/event/domain.rs` | 运行时观测：UI/SSE/telemetry | 前端 SSE, 遥测 |
| `StorageEvent` | `crates/core/src/event/types.rs` | 持久化：replay/cursor/session 恢复 | `FileSystemSessionRepository` |

`EventTranslator`: `crates/core/src/event/translate.rs` — 做 `StorageEvent` → `AgentEvent` 投影

持久化实现：
- `EventLog` (append-only JSONL): `crates/storage/src/session/event_log.rs`
- `FileSystemSessionRepository`: `crates/storage/src/session/repository.rs`

### 3. Approval 通过专门 broker 处理

当 policy 决策返回"需要审批"时：
- runtime 通过专门的 approval broker 挂起并等待结果
- event 层只镜像 `ApprovalRequested` / `ApprovalResolved`
- broker 接口显式接收 turn cancellation，避免审批挂起与中断语义脱节

审批**不通过 EventBus** 直接完成 request / response。

实现位置：
- Trait: `crates/runtime/src/approval_service.rs` — `ApprovalBroker` trait
- 默认实现: `DefaultApprovalBroker` — 根据 `ApprovalRequest.default` 立即给出 allow/deny
- 集成: `crates/runtime/src/agent_loop/tool_cycle.rs` — tool call 执行路径接入三态分支

### 4. Durable session events 与 runtime observation events 可以不同

AstrCode 同时保留：
- 面向 replay / cursor / session 恢复的 durable event (`StorageEvent`)
- 面向实时观察者的 runtime agent event (`AgentEvent`)

二者通过 `EventTranslator` 投影，但不强制等同。

`StoredEvent { storage_seq, event: StorageEvent }` 是 append-only JSONL 的最终持久化格式，
SSE 事件 id 形如 `{storage_seq}.{subindex}`。

## Consequences

正面影响：
- 权限、审批、context pressure 有了正式控制面
- UI 和 transport 不再被误当成执行仲裁者
- runtime event 可以为多客户端和多协议接入自然复用
- session log 与 runtime observation 的分层更清晰

代价：
- runtime 需要新增 approval broker 一类的显式服务
- 需要定义 policy input / decision 与 event taxonomy
- 一些现有事件流和持久化事件之间需要重新梳理投影关系

## Current Implementation Status (2026-04-03)

### Policy 控制面 ✓

- `crates/core/src/policy/` — `PolicyEngine` trait 与 `PolicyVerdict<T>` 三态 (Allow/Deny/Ask)
- `ModelRequest` / `CapabilityCall` / `ContextPressureInput` / `ContextStrategyDecision` 已冻结为正式契约
- `AllowAllPolicyEngine` 作为默认实现

### Approval Broker ✓

- `crates/runtime/src/approval_service.rs` — `ApprovalBroker` trait 与 `DefaultApprovalBroker`
- `DefaultApprovalBroker` 根据 `ApprovalRequest.default` 立即给出 allow/deny
- broker 接口显式接收 `CancelToken`

### AgentLoop 集成 ✓

- `crates/runtime/src/agent_loop.rs` — `AgentLoop` 持有 `policy: Arc<dyn PolicyEngine>` 和 `approval: Arc<dyn ApprovalBroker>`
- `crates/runtime/src/agent_loop/tool_cycle.rs` — tool call 三态分支
- `crates/runtime/src/agent_loop/turn_runner.rs` — turn 开始时接入 `check_model_request()` 策略检查

### Event 观测面 ✓

- `crates/core/src/event/domain.rs` — `AgentEvent` + `Phase` 枚举
- `crates/core/src/event/types.rs` — `StorageEvent` (含 `PromptMetrics`, `CompactApplied`, `TurnDone.reason`)
- `crates/core/src/event/translate.rs` — `EventTranslator` 投影
- `crates/storage/src/session/event_log.rs` — append-only JSONL
- `crates/storage/src/session/repository.rs` — `FileSystemSessionRepository`

### 已完成 (2026-04-03)

- P3 (上下文压缩): compaction / microcompact 模块已实现，`CompactApplied` 事件 + `PromptMetrics` 事件已暴露
- Token usage 统计: `crates/runtime/src/agent_loop/token_usage.rs`
