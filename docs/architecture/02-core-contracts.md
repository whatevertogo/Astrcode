# Core Contracts

## Goal

Layer 1 的设计目标只有一个：

让后续功能迭代尽量不再改动核心执行语义。

因此核心只冻结四类契约：

- AgentLoop Contract
- Capability Contract
- Policy Contract
- Event Contract

## 1. AgentLoop Contract

AgentLoop 是唯一固定的执行语义。

`Turn` 是 AgentLoop 的基本调度单位。  
一次用户提交、一次审批挂起/恢复、一次中断、一次 turn 结束，都是围绕同一个 turn 边界发生。

当前 AstrCode 已经有相对清晰的雏形：

- `crates/runtime/src/agent_loop.rs`
- `crates/runtime/src/agent_loop/turn_runner.rs`
- `crates/runtime/src/agent_loop/tool_cycle.rs`
- `crates/core/src/runtime/traits.rs`

未来应继续保持其最小职责：

1. 构造模型请求
2. 调用模型
3. 遍历 capability call
4. 执行同步策略判断
5. 执行动作
6. 发出异步事件
7. 处理上下文压力与结束条件

AgentLoop 通过一个最小的模型调用端口工作，可以把它理解为 `ModelCall` trait：

- 输入是消息、系统提示、工具定义和取消信号
- 输出是 token / delta 流与最终模型响应

这个端口只定义“如何完成一次模型调用”，不定义：

- provider registry
- API key / 凭据管理
- model discovery
- provider failover / routing

这些都属于 Layer 2 的 runtime assembly 问题。  
当前实现里，这个端口的现实落点是 `crates/runtime/src/llm/mod.rs` 中的 `LlmProvider`。

AgentLoop 不应直接承担：

- transport
- plugin lifecycle
- storage backend 选择
- provider registry 发现
- skills 文件扫描
- MCP / ACP 接入

### Recommended Pseudocode

```rust
loop {
    emit(TurnStarted)

    let request = planner.build_request(&state);
    let request = policy.check_model_request(request, &policy_ctx).await?;

    emit(ModelRequestPrepared)
    let response = model_call.generate(request).await?;
    emit(ModelResponseCompleted)

    for call in response.capability_calls {
        match policy.check_capability_call(call.clone(), &policy_ctx).await? {
            PolicyVerdict::Allow(updated_call) => {
                emit(CapabilityInvokeStarted)
                let result = router.invoke(updated_call, &cap_ctx).await?;
                emit(CapabilityInvokeCompleted)
                state.apply(result);
            }
            PolicyVerdict::Deny { reason } => {
                emit(CapabilityInvokeDenied { reason })
                state.apply_denial(reason);
            }
            PolicyVerdict::Ask(pending) => {
                let resolution = approval.request(pending.request).await?;
                emit(ApprovalResolved)
                state.apply_approval_resolution(resolution);
            }
        }
    }

    if state.near_limit() {
        emit(ContextPressure)
        match policy.decide_context_strategy(state.context_pressure(), &policy_ctx).await? {
            ContextStrategyDecision::Compact => { ... }
            ContextStrategyDecision::Summarize => { ... }
            ContextStrategyDecision::Truncate => { ... }
            ContextStrategyDecision::Ignore => {}
        }
    }

    if response.stop_reason == StopReason::EndTurn {
        emit(TurnEnded)
        break;
    }
}
```

### Turn Is the Execution Anchor

Turn 不是附属细节，而是四个契约交汇的锚点：

- AgentLoop 按 turn 调度
- Policy 按 turn 做前置决策
- Event 按 turn 发射和关联
- Capability 在 turn 内执行并把结果写回同一段上下文

因此，未来即使引入 workflow、subagent、approval broker 或更复杂的 compaction，也应尽量保持“turn 是最小执行单元”这个事实不变。

## 2. Capability Contract

Capability 是唯一一等动作模型。

不要同时维护：

- tool 动作模型
- provider 动作模型
- workflow step 动作模型
- plugin action 动作模型

它们最终都应该落回 capability。

### Recommended Shape

```rust
trait Capability: Send + Sync {
    fn descriptor(&self) -> CapabilityDescriptor;
    async fn invoke(
        &self,
        call: CapabilityCall,
        ctx: CapabilityContext,
    ) -> CapabilityResult;
}
```

### Kind Is Metadata, Not a Second Protocol

Layer 1 只要求 capability 是一个 `{ descriptor, invoke }` 对。  
`CapabilityKind` 是路由、策略、展示和适配时使用的元数据，不是第二套调用协议。

这意味着：

- 新 capability source 不应该因为新增 `kind` 就要求改写 AgentLoop
- workflow、memory、context、resource 之类能力仍然走同一个 invoke contract
- `kind` 更适合被 Layer 2 用于策略、投影和 transport 适配

当前代码里，`CapabilityKind::Tool` 仍会被某些 adapter surface 特判，例如：

- 把 capability 投影成 LLM tool definitions
- 判断某个 capability 是否允许走 tool-call 执行路径

这属于“工具调用适配面”的实现细节，不代表 Layer 1 需要按 `kind` 分裂执行语义。

当前代码里最接近这一抽象的是：

- `CapabilityDescriptor`
- `CapabilityInvoker`
- `CapabilityRouter`

相关代码：

- `crates/core/src/capability.rs`
- `crates/core/src/registry/router.rs`

### Naming Taxonomy

建议 capability name 使用稳定的、可审计的命名空间：

- `model.generate`
- `tool.fs.read`
- `tool.fs.edit`
- `tool.shell.exec`
- `memory.search`
- `session.compact`
- `skill.load`

`kind` 只是分类标签，不是第二套协议。

### Consequences

好处：

- 权限检查统一
- schema 暴露统一
- 事件审计统一
- 内置能力和插件能力进入同一路由

代价：

- `ToolRegistry` 只能退化为 builtin capability source
- provider 如果继续保持专用接口，需要做 adapter 接入 capability 世界

## 3. Policy Contract

Policy 是唯一同步决策入口。

这里的“同步”不是说实现上不能 `await`，而是说它拥有改变执行结果的权力。  
Policy 决定“某个动作能不能发生”，而 Event 只负责观察。

### Recommended Shape

```rust
trait PolicyEngine: Send + Sync {
    async fn check_model_request(
        &self,
        request: ModelRequest,
        ctx: &PolicyContext,
    ) -> Result<ModelRequest>;

    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>>;

    async fn decide_context_strategy(
        &self,
        input: ContextPressureInput,
        ctx: &PolicyContext,
    ) -> Result<ContextStrategyDecision>;
}

enum PolicyVerdict<T> {
    Allow(T),
    Deny { reason: String },
    Ask(ApprovalPending<T>),
}
```

这里刻意没有使用“万能 hook”。

原因：

- 模型请求改写和工具执行审批是同类问题，都是“前置决策”
- compaction 不是普通 hook，更接近 strategy decision point
- 如果把它们都塞进一个通用 hook trait，职责会很快混乱

这不等于 SDK 里不能存在插件本地的 hook 组合工具。  
插件作者完全可以在自己的进程内用轻量 hook 做局部校验、白名单或复用逻辑；只是这些工具不应该被误认为宿主 runtime 的核心 Policy contract。

### Approval Is Not EventBus

`Ask` 不是通过广播总线来等待结果，而是通过专门的 `ApprovalBroker` 之类的运行时服务完成。

也就是说：

- Policy 负责产出 `Ask`
- Approval service 负责挂起与恢复
- Event 只负责镜像 `ApprovalRequested` / `ApprovalResolved`

## 4. Event Contract

Event 是唯一异步观测面。

它只表达“发生了什么”，不表达“下一步该怎么做”。

### Recommended Shape

```rust
trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);
}

enum AgentEvent {
    SessionStarted,
    SessionEnded,

    TurnStarted { turn_id: String },
    TurnEnded { turn_id: String, stop_reason: StopReason },

    ModelRequestPrepared,
    ModelStreamDelta,
    ModelResponseCompleted,

    CapabilityInvokeStarted { name: String },
    CapabilityInvokeCompleted { name: String },
    CapabilityInvokeDenied { name: String, reason: String },

    ApprovalRequested,
    ApprovalResolved,

    ContextPressure { used: u32, limit: u32 },
    ContextCompacted,

    AgentError { stage: String, message: String },
}
```

### Event vs Persistent Session Log

AstrCode 当前已有 `StorageEvent` 体系用于会话持久化和 SSE replay。  
未来建议逻辑上区分两类事件：

- `AgentEvent`：运行时异步观测事件，面向 UI、telemetry、CLI、debugging
- `StorageEvent`：持久化事件，面向 replay、session 恢复、SSE cursor

两者可以互相投影，但不应完全等同。

这样可以避免：

- 为了 UI 增加的瞬时事件污染持久化模型
- 为了 replay 稳定性而把所有运行时瞬时事件都强行持久化

## Current Status Assessment

当前仓库的现实情况是：

- `AgentLoop` 基本清晰
- `Capability` 已经是正确方向
- `Policy` 还没有收敛成正式契约
- `Event` 目前更多以 `StorageEvent` 形式存在，缺少专门的 runtime observation contract

因此，后续的重构重点不在重写 loop，而在补齐 Policy 与 Event 这两个契约。
