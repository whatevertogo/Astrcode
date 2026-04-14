## Context

现在的 `session-runtime` 已经有：

- `LlmFinishReason::MaxTokens`
- `warn_if_output_truncated(...)`
- `max_output_continuation_attempts` 配置

但缺少真正的恢复路径。结果是：

- assistant 输出会被截断
- turn loop 不会自动补一轮 continuation
- summary 也无法说明这次 turn 到底经历了几次截断恢复

从架构上看，这不是 `application` 的职责，因为是否恢复取决于 turn 内部的 assistant 输出、tool call 情况和 continuation 次数；也不是 `kernel` 的职责，因为它不持有单 session 执行真相。

## Goals / Non-Goals

**Goals:**

- 为 `max_tokens` 截断建立正式的 turn 内恢复路径
- 让恢复路径使用稳定的 synthetic prompt origin，而不是复用错误语义不匹配的现有 origin
- 让恢复次数与终止原因进入稳定汇总
- 保持现有 `request -> llm_cycle -> tool_cycle` 编排结构不塌陷

**Non-Goals:**

- 不改变 provider 侧 `LlmFinishReason` 的定义
- 不在这次 change 里引入新的外部协议事件
- 不把带 tool calls 的 assistant 截断恢复成多轮 speculative 执行
- 不在这次 change 里处理“输出太长但不是 `max_tokens`”的其他恢复策略

## Decisions

### D1: 截断恢复使用专门的 synthetic continuation prompt

本次 change 会为“从截断处继续”引入单独的 synthetic prompt 语义，例如新的 `UserMessageOrigin` 枚举值，而不是复用：

- `CompactSummary`
- `ReactivationPrompt`
- 普通 `User`

这样可以保持事件与消息协议的语义清晰，也能让 request assembly / observability 精确识别这类恢复。

### D2: 恢复只在“无 tool call 的截断 assistant 输出”上触发

当 assistant 输出被 `max_tokens` 截断时，只有在当前输出不包含 tool calls 的情况下才自动进入 continuation 恢复。  
理由是带 tool call 的截断输出在语义上可能仍处于不完整的工具规划中，贸然续写会把“继续文本生成”和“继续工具编排”混在一起，增加不必要风险。

### D3: 恢复尝试次数与最终失败释放时机由 `session-runtime` 内部状态严格限制

使用现有 `max_output_continuation_attempts` 作为硬上限。  
`TurnExecutionContext` 持有当前已尝试次数；每次成功注入 continuation prompt 都计数；达到上限后 turn 以明确 stop cause 结束，而不是继续告警但放任输出中断。

这意味着处于“仍可恢复”的 `MaxTokens` 截断不会立刻被当作最终失败对外释放；只有当：

- continuation 尝试达到上限
- 当前输出含有 tool calls
- turn 已取消或命中其他禁止恢复条件

系统才把该场景收束成最终停止路径。

### D4: 恢复逻辑保持为 turn 子域的独立模块

本次 change 倾向增加 `turn/continuation_cycle.rs` 或等价模块，由它负责：

- 判断是否允许恢复
- 生成 synthetic continuation prompt
- 返回 continue 所需的状态更新

这样可以避免把新的恢复分支继续塞进 `step.rs`，保持与 `compaction_cycle` 对称的结构。

### D5: 第一阶段不做 provider 侧 max-output 升档

Claude Code 在部分场景下会先尝试提高 `max_output_tokens` 再决定是否转入 prompt-level continuation。Astrcode 第一阶段不引入这条路径，原因是：

- 这会把 provider-specific 策略带进 `session-runtime` 的通用 loop
- 当前仓库真正缺的是“恢复闭环”而不是“provider 参数调优”
- 先把 transition、prompt origin 和恢复上限做稳定，再评估是否值得追加 provider 级优化

## Risks / Trade-offs

- [Risk] continuation prompt 写得不好，可能导致模型重复、道歉或总结
  - Mitigation：把 prompt 语义限定为“直接继续，不重复，不回顾”，并通过测试固定行为约束

- [Risk] 连续截断后自动续写过多，造成超长 turn
  - Mitigation：使用硬上限 `max_output_continuation_attempts`，并把停止原因纳入汇总

- [Risk] 中间截断被暂缓释放后，诊断上看起来像“没有错误”
  - Mitigation：在 turn summary / observability 中明确区分“发生过截断但已恢复”与“最终因截断停止”

- [Risk] 新的 message origin 增加 projector / token 预算 / replay 兼容工作
  - Mitigation：沿用现有 `CompactSummary` / `ReactivationPrompt` 的处理模式，完整梳理 message -> event -> projection 链路

## Migration Plan

1. 在 `core` 中增加截断恢复所需的 synthetic prompt 语义
2. 在 `session-runtime/turn` 中新增 continuation 恢复模块
3. 让 `step.rs` 在 `MaxTokens` 场景下调用该模块，而不是只记 warning
4. 把恢复次数与停止原因汇入 `TurnSummary`
5. 增加测试覆盖“允许恢复”“达到上限”“存在 tool calls 时不恢复”“中间截断在恢复耗尽前不作为最终失败释放”

回滚策略：

- 如果恢复 prompt 效果不理想，可暂时关闭自动恢复分支，退回 warning-only，但保留新增的类型与测试骨架

## Open Questions

- continuation prompt 是否需要进入 durable event log，还是只作为内部 synthetic message 参与本次 turn 的 request 组装？
- 当 assistant 同时包含少量文本和未完整闭合的 tool planning 痕迹时，是否一律视为“不做自动恢复”？
