## Context

现在 `llm_cycle` 已经能接收 provider 发来的 `ToolCallDelta`，但它在 `emit_llm_delta_live(...)` 中直接忽略了这类事件。与此同时，`tool_cycle` 只接受完整的 `Vec<ToolCallRequest>`，因此工具调度天然发生在完整 `LlmOutput` 之后。

这与博客里最有价值的工程优化点有明显差距：对于一串只读工具调用，系统完全可以在模型还没结束整条 assistant 输出之前就开始做安全工作，从而缩短 step 的墙钟时间。

但需要注意，Claude Code 的 `StreamingToolExecutor` 并不是对任意残缺 delta 直接执行工具；它消费的是已经形成稳定 identity 的 streamed `tool_use` 块，并在 fallback、discard、取消场景下做额外保护。Astrcode 当前 provider 层暴露的是 `ToolCallDelta`，因此必须先在 `session-runtime` 内部完成“可执行候选组装”，再交给调度器，而不是把半成品参数直接塞进 `tool_cycle`。

## Goals / Non-Goals

**Goals:**

- 在不破坏现有模块边界的前提下，把 `ToolCallDelta` 纳入 turn 内的正式调度信息流
- 支持只读、安全、参数已闭合工具的提前执行
- 保证 durable 事件顺序仍然可回放、可理解
- 为是否真的产生重叠收益提供可观测指标

**Non-Goals:**

- 不对副作用工具做 speculative 执行
- 不在这次 change 里重写 provider streaming 协议
- 不把 `tool_cycle` 改成一个持久常驻 actor 系统
- 不改变 `kernel` 对能力并发安全的职责边界

## Decisions

### D1: `llm_cycle` 输出“流式工具片段”，runner 负责组装可执行候选并持有调度状态

`llm_cycle` 负责把 provider 发来的 `ToolCallDelta` 传成更适合 turn 使用的流式输入，但不直接执行工具。  
`runner/step` 内新增 step-local assembler / planner，把同一调用的多个 delta 组装成“可执行候选”；真正的调度状态也由它持有，这样：

- `llm_cycle` 仍然专注于 LLM streaming
- `tool_cycle` 仍然专注于工具调用
- “什么时候允许提前执行”这一策略仍然留在 `session-runtime/turn`
- scheduler 看到的是稳定候选，而不是任意半成品 delta

### D2: 只对 fully-formed + stable-identity + concurrency-safe 工具做提前执行

提前执行必须是保守优化，而不是 speculative 赌博。  
因此第一版只允许满足以下条件的调用提前开始：

- 工具参数已经形成可解析的完整输入
- tool call identity 已稳定，不再依赖后续 delta 改写调用含义
- capability spec 声明 `concurrency_safe = true`
- 不依赖完整 assistant 文本上下文才能判断是否合法

写操作、参数未闭合或需要完整 assistant 语义辅助判断的调用，一律回退到现有完整输出后执行。

### D3: 提前执行结果进入 step-local 缓冲，durable 顺序晚于 assistant 定稿

为保持 event log 的可读性与回放稳定性，提前执行的工具即使在墙钟时间上先完成，其 durable 事件顺序仍应服从 step 语义：

1. assistant 输出定稿
2. 对应 tool call / tool result durable 事件写入

提前执行阶段只负责：

- 启动真实工具工作
- 收集 step-local 结果与 live delta

而 durable flush 统一放在 assistant 定稿之后的收口点完成。

### D4: scheduler 必须定义 discard / fallback / cancel 语义

Claude Code 在 streaming fallback、兄弟调用失败、用户中断等场景下，会把提前执行结果标记为 discard 或生成受控错误结果，而不是让“已经跑出去的工作”直接污染最终语义。Astrcode 第一版也需要有对应的 step-local 收口规则：

- 如果后续 delta 使某个候选不再满足稳定性条件，scheduler SHALL 放弃其提前执行资格
- 如果 assistant 最终输出与已组装候选不一致，提前执行结果 SHALL 被 discard，不进入 durable 事件
- 如果 turn 被取消或 step 被中断，scheduler SHALL 统一走取消/丢弃路径，而不是局部泄漏半完成结果

### D5: observability 记录重叠执行而不是只记录最终结果

仅靠最后的 tool result 数量无法判断这项优化是否产生收益。  
因此 observability 需要增加“多少调用被提前执行”“LLM/tool 有多少毫秒重叠”“多少调用因参数未闭合或副作用被保守回退”等诊断数据。

## Risks / Trade-offs

- [Risk] 提前执行破坏现有 durable 事件顺序，导致回放难以理解
  - Mitigation：严格区分 live 执行时间顺序与 durable append 顺序，统一在 assistant 定稿后 flush

- [Risk] 参数闭合或 identity 稳定性判断不稳，造成过早执行
  - Mitigation：第一版只接受完整 JSON 可解析且调用 identity 已稳定的工具输入，宁可少做，不做猜测

- [Risk] 工具结果提前完成后被取消，清理路径复杂
  - Mitigation：把提前执行结果收敛到 step-local 缓冲中，由 step 收口逻辑统一决定 flush 或丢弃

## Migration Plan

1. 让 `llm_cycle` 暴露可消费的流式 tool delta 输入
2. 在 `runner/step` 中加入 step-local 的 tool call assembler + streaming planner
3. 为 `tool_cycle` 增加“提前执行 + 延后 durable flush + discard/fallback”的收口能力
4. 增加重叠执行 observability
5. 先只对白名单场景启用，再逐步扩大覆盖面

回滚策略：

- 保留 delta 解析与 planner 代码，但关闭提前执行开关，退回到完整输出后统一执行

## Open Questions

- streaming planner 是否需要为不同 provider 抽象统一的“参数闭合 + identity 稳定”判定接口，还是先在 `session-runtime` 内做通用 JSON/append-only 判断？
- 当 assistant 在后续 delta 中撤回或覆盖先前 tool 计划时，第一版是直接 discard 已执行结果，还是完全禁止此类调用提前执行？
