# ADR-0006: Turn 状态机化——显式 TurnOutcome 与移除 max_steps

**状态**: 已实施
**日期**: 2026-04-02
**实施提交**: d690778

---

## 背景

`AgentLoop::run_turn()` 原来返回 `Result<()`，turn 的终止原因完全隐含在 `StorageEvent` 流中：调用方只知道"成功"或"panic/error"，无法区分自然完成、用户取消、不可恢复错误三种语义。

同时，`AgentLoop` 持有 `max_steps: Option<usize>` 作为防止无限循环的安全网，但这是错误的粒度：它在正常工作流中截断 agent（发出 TurnDone 而非 Error），既会中断合法的长任务，又不能真正防止 token 爆炸——token 数量与 step 数无直接关系。

---

## 决策

### 1. 引入 `TurnOutcome` 枚举，`run_turn()` 返回 `Result<TurnOutcome>`

```rust
pub enum TurnOutcome {
    /// LLM 返回纯文本（无 tool_calls），自然结束
    Completed,
    /// 用户取消或 CancelToken 触发
    Cancelled,
    /// 不可恢复错误
    Error { message: String },
}
```

- 所有终止路径仍然通过 `on_event` 发出 `TurnDone` 事件（SSE 客户端不变）
- `TurnOutcome` 是纯值，调用方（`RuntimeService`）可自由 match 决定后续行为
- `finish_turn` / `finish_with_error` / `finish_interrupted` 统一返回 `Result<TurnOutcome>`

### 2. `TurnDone` 携带 `reason` 字段

```rust
StorageEvent::TurnDone {
    turn_id: Option<String>,
    timestamp: DateTime<Utc>,
    reason: Option<String>,  // "completed" | "cancelled" | "error"
}
```

- `reason` 带 `#[serde(default)]`，旧 JSONL 文件反序列化为 `None`，向后兼容
- `finish_interrupted` 产生 `reason = "cancelled"`（不再复用 `finish_with_error`）

### 3. 移除 `max_steps`

- 删除 `AgentLoop.max_steps` 字段、`with_max_steps()` 方法、`reached_max_steps()` 函数
- agent 的自然终止完全依赖：LLM 返回纯文本（无 tool_calls），或用户触发 CancelToken
- **真正的安全网**由上下文压缩（roadmap P3）承担：消息历史超出阈值时自动压缩，token 不会无限增长

---

## 核心思想

**终止原因是业务语义，不是实现细节。**

`Completed / Cancelled / Error` 三种终止对上层有完全不同的含义：
- `Completed` → 任务正常完成，可触发后续 agent 或通知用户
- `Cancelled` → 用户主动中断，不计入失败指标，不需要重试
- `Error` → 系统或 LLM 失败，可能需要告警或重试

将三者都隐藏在 `Result<()>` 里，调用方被迫从事件流推断原因，既脆弱又耦合。显式枚举让调用方的意图清晰，也为未来的多 agent 编排提供干净的信号接口——编排层只需 match `TurnOutcome` 就能做路由决策，无需理解 agent 内部事件流。

**`max_steps` 是错误层次的安全网。**

防止无限循环的正确位置是"消息体积"（token），而非"调用次数"（step）。step 数限制会误伤合法的长任务（如批量文件处理），同时对真正危险的场景（每步输出极少 token 的循环）无效。移除后，安全网下移到上下文压缩层，在正确的层次解决正确的问题。

## 与多 Agent 编排的关系

`TurnOutcome` 是编排层的自然输入接口。未来插件/工具编排 agent 时：

```
编排层
  ↓ 调用 run_turn()
AgentLoop（单次 turn 执行器）
  ↓ 返回 TurnOutcome
编排层 match outcome → 决定是否启动下一个 agent
```

`AgentLoop` 的边界保持为单次 turn 执行器，不感知编排逻辑。`TurnOutcome` 未来可按需扩展（如 `Completed { final_message }` 携带输出），但当前保持最小化以避免过早设计。


## Current Implementation Status

截至 2026-04-03，已全部落地：

- `TurnOutcome` 枚举: `crates/runtime-agent-loop/src/agent_loop.rs`
- `run_turn()` 返回 `Result<TurnOutcome>`: `crates/runtime-agent-loop/src/agent_loop.rs`
- `max_steps` 已完全移除
- `TurnDone.reason` 字段: `crates/core/src/event/types.rs` — `reason: Option<String>` 带 `#[serde(default)]` 向后兼容
- `finish_turn` / `finish_with_error` / `finish_interrupted`: `crates/runtime-agent-loop/src/agent_loop.rs`
- ADR-0006 实施提交: d690778
