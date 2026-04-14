## Context

当前 `run_turn` 与 `run_single_step_with` 已经把 Rust 版本的 agent loop 拆成了清晰的模块边界：

- `turn/request` 负责 prompt 组装
- `turn/llm_cycle` 负责流式 LLM 调用
- `turn/tool_cycle` 负责工具执行
- `turn/compaction_cycle` 负责 prompt-too-long 恢复

但 loop 仍然缺一个稳定的“状态过渡语义层”。现在系统知道某一轮是 `Continue`、`Completed` 还是 `Cancelled`，却不知道这次 `Continue` 到底来自：

- tool 结果已经追加完成
- reactive compact 成功恢复
- budget 允许继续
- 未来的 `max_tokens` 截断恢复

这会带来两个问题：

1. 现有 archived spec 里写下的 auto-continue 需求很难自然落地；
2. 后续扩展新恢复路径时，代码会继续沿着“再加一个局部布尔值/计数器”的方向变脆。

Claude Code `query.ts` 已经证明，“显式 transition reason” 是把 agent loop 做稳的关键抽象；但 Astrcode 不该因此回退到单个巨型循环。我们要借鉴的是状态语义，不是代码形态。

## Goals / Non-Goals

**Goals:**

- 在 `session-runtime/turn` 内建立显式的 transition / stop cause 语义
- 让 budget-driven auto-continue 具备稳定的 loop 挂接点
- 让 turn summary / observability 能基于稳定原因输出，而不是扫描和猜测
- 保持 `application`、`kernel`、`protocol` 边界不变

**Non-Goals:**

- 不引入新的外部协议字段或 HTTP/SSE DTO 变更
- 不在这次 change 里实现 `max_tokens` 截断恢复本身
- 不改写现有 request assembly、micro compact、tool cycle 的主职责
- 不把 turn loop 改回单个巨型函数

## Decisions

### D1: 在 `session-runtime/turn` 中引入显式 transition / stop cause 类型

本次 change 将在 `session-runtime` 内部新增稳定的 loop 语义类型，例如：

- `TurnLoopTransition`
- `TurnStopCause`

它们是 `turn` 子域内部真相的一部分，不进入 `protocol`，也不要求 `application` 直接理解其细节。

这样做比继续增加 `reactive_compact_attempts`、`auto_compaction_count` 这类离散字段更稳，因为新的 loop 能力会共享同一套过渡语义，而不是继续长出平行状态。  
它本质上是把 Claude Code 里的 `transition.reason` 思想，翻译成符合 Astrcode 架构的内部类型系统。

### D2: transition 记录在 `TurnExecutionContext`，而不是由外层治理层推断

`session-runtime` 是单 session 执行真相面，continue / stop 的原因天然属于这里。  
因此 transition 与 stop cause 由 `TurnExecutionContext` 持有，并由 runner/step 在边界点更新：

- request 组装后是否进入下一轮
- LLM 输出后是否自然结束
- tool 结果追加后为何继续
- reactive compact 成功后为何重试

治理层只消费稳定汇总结果，不反向参与判断。

### D3: budget-driven auto-continue 与 transition 模型同时落地

现有 spec 已经把 auto-continue 写成 turn-orchestration/turn-budget-governance 的正式能力。  
本次 change 不再把 budget 视为将来再接的附加逻辑，而是把它作为首个正式 transition source 纳入模型：

- `BudgetAllowsContinuation`
- `BudgetStopsContinuation`

这样后续截断恢复和工具流式调度可以复用同一套模式，不需要再为每条路径重新定义状态形状。

### D4: 模块化 runner 保持不变，transition 只补语义骨架

`request -> llm_cycle -> tool_cycle -> compaction_cycle` 这条模块化流水线继续保留。  
transition 模型的职责是：

- 给 `runner` 和 `step` 一个统一的 continue / stop 语义骨架
- 让新能力有稳定挂接点
- 避免局部布尔值和计数器继续膨胀

它不是把这些模块重新折叠回一个巨型 `queryLoop()`。

### D5: turn summary 暴露结构化原因，但 durable 事件仍然是原始事实源

`PromptMetrics`、`AssistantFinal`、`TurnDone`、`CompactApplied` 继续承担原始事实源职责。  
`TurnSummary` 在其上增加结构化的 transition / stop cause 聚合字段，供治理和诊断使用。

这符合当前仓库“原始事件 + 稳定汇总分离”的方向，也避免把业务逻辑塞进 DTO 或 server handler。

## Risks / Trade-offs

- [Risk] transition 类型过早设计过重，反而限制后续演进
  - Mitigation：第一版只覆盖已经存在或已确认要落地的路径，不为假想能力预留过宽枚举

- [Risk] auto-continue 接线后，loop 语义比现在复杂，测试面增大
  - Mitigation：围绕 `run_single_step_with` 与 `run_turn` 增加 transition-oriented 单元测试，而不是只测最终消息结果

- [Risk] turn summary 暴露原因后，后续字段变更会影响 observability 使用方
  - Mitigation：先把原因建模在 `session-runtime` 内部稳定汇总中，等实现和消费路径稳定后再考虑是否外露更多字段

## Migration Plan

1. 在 `session-runtime/turn` 内增加 transition / stop cause 类型
2. 让 `TurnExecutionContext` 和 `TurnSummary` 接入这些类型
3. 在 `runner/step` 中把现有 continue/end 分支改成更新显式原因
4. 把 budget-driven auto-continue 接到这套模型
5. 为 summary / observability 和 loop 状态增加测试

回滚策略：

- 如果 transition 建模影响过大，可以先保留类型定义与 summary 字段，临时把 runner 逻辑回退到现有分支推进方式
- 因为不改外部协议，回滚不会影响 `application` 或 `server` 的调用入口

## Open Questions

- `TurnStopCause` 是否应该直接并入现有 `TurnFinishReason`，还是单独作为更细粒度的原因字段保留？
- budget-driven auto-continue 是否应该在 `TurnDone` durable 事件里同时写入 reason，还是只先进入 `TurnSummary`？
