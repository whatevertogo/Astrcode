# ADR-0006: Turn 状态机化——显式 TurnOutcome 与移除 max_steps

- Status: Accepted
- Date: 2026-04-02

## Context

`AgentLoop::run_turn()` 过去只返回 `Result<()>`，调用方无法直接区分自然完成、用户取消和不可恢复错误，只能从事件流推断终止原因。同时，`max_steps` 用 step 数作为安全网，会误伤合法长任务，却不能真正控制 token 膨胀。

## Decision

把 turn 的终止结果建模为显式业务语义，并移除 step 数上限。

- `run_turn()` 返回 `Result<TurnOutcome>`，终止结果至少区分 `Completed`、`Cancelled` 和 `Error`。
- `TurnDone` 事件显式记录终止原因，便于持久化和上层消费统一理解 turn 结果。
- 移除 `max_steps`；turn 的停止条件依赖自然完成、取消信号和上下文控制，而不是固定 step 上限。

## Consequences

- 调用方可以直接根据 `TurnOutcome` 做编排、重试和告警决策，而不必倒推事件流。
- turn 终止语义从实现细节提升为正式契约。
- 安全网从错误的 step 粒度转向更合适的上下文和 token 控制粒度。
