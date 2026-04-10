# ADR-0006: Turn State Machine — Explicit TurnOutcome and Remove max_steps

- Status: Accepted
- Date: 2026-04-02

## Context

`AgentLoop::run_turn()` 过去只返回 `Result<()>`，调用方无法直接区分自然完成、用户取消和不可恢复错误。与此同时，`max_steps` 作为安全网只用 step 计数限制执行，会误伤合法长任务且无法反映真实 token/上下文约束。

## Decision

把 turn 终止结果建模为显式业务语义，并移除 step 数上限。

- `run_turn()` 返回 `Result<TurnOutcome>`，至少区分 `Completed`、`Cancelled`、`Error`。
- `TurnDone` 事件显式记录终止原因，供持久化和上层消费统一理解 turn 结果。
- 从当前运行时设计中移除 `max_steps`，turn 的停止条件依赖自然完成、用户取消、策略/预算触发和上下文控制，而不是固定 step 上限。

## Consequences

- 调用方可以直接根据 `TurnOutcome` 做编排、重试和告警判断。
- turn 终止语义从实现细节提升为正式契约。
- 安全网从 step 数控制转向更适合的上下文/预算控制粒度。
