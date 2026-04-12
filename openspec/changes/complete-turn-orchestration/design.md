## Context

当前 `session-runtime` 已经承接了单 session 真相与 turn 执行主路径，`run_turn` 内部也已经具备多 step 的 LLM -> 工具 -> LLM 循环、微压缩、裁剪和自动压缩能力。但与旧 runtime 相比，还有三块关键缺口：

- token budget 已有工具函数，但没有真正驱动 auto-continue
- prompt metrics 目前仍以事件形式存在，没有形成 turn 级稳定汇总
- compaction tail 目前主要通过 `recent_turn_event_tail` 表达，是否需要显式快照尚未定案

该 change 只处理 `session-runtime` 内的执行闭环，不改变 `kernel` 的全局控制职责，也不把用例编排重新推回 `application`。

## Goals / Non-Goals

**Goals:**

- 在 `session-runtime` 内补齐 token budget 驱动的 auto-continue 闭环
- 为 turn 执行沉淀稳定的 observability 汇总结果
- 明确 compaction tail 的正式契约与最小实现形态
- 让 `App::submit_prompt` 继续保持“只发起 turn”的边界

**Non-Goals:**

- 不重写现有 request assembly 管线
- 不新增 runtime façade 或全局 loop surface
- 不把 observability 聚合放进 `application`
- 不修改 HTTP/SSE DTO 结构

## Decisions

### D1: token budget 逻辑留在 `session-runtime/turn`

继续/停止的判断依赖 turn 内 token 使用量、continuation 次数和本轮输出增量，这些都是单 session 执行真相，因此必须留在 `session-runtime/turn`。  
备选方案是把 continue 决策提到 `application`，但这会让 `application` 知道过多 turn 内细节，破坏当前边界。

### D2: `submit_prompt` 只负责启动，不负责续写策略

`application::App::submit_prompt` 继续只做参数校验、配置读取和调用 `SessionRuntime`。  
真正的 auto-continue nudge 注入、上限判断和 turn 终止条件都放在 `run_turn` 内。  
这样做可以保证所有 session turn 的策略一致，不会出现不同入口分裂出不同执行语义。

### D3: observability 先在 `session-runtime` 内形成聚合结果，再由治理层消费

turn 级指标的原始来源在执行循环内部，因此先由 `session-runtime` 收敛成稳定结构，再由 `application/governance` 消费最合适。  
备选方案是直接在 `application` 里监听 `PromptMetrics` 事件做聚合，但这会把业务用例层变成事件处理中心，不利于长期维护。

### D4: compaction tail 优先复用现有能力，而不是为对齐旧名词额外造层

如果 `recent_turn_event_tail` 和当前 `recent_stored_events` 已能满足自动压缩与恢复需要，就把这套能力固化进 spec；只有在现有表达不足时，才新增显式 `CompactionTailSnapshot`。  
这样可以避免“为了迁旧功能名词而增加一层空包装”。

## Risks / Trade-offs

- [Risk] auto-continue 判断不稳，造成意外长循环
  - Mitigation：引入硬上限 `max_continuations`，并把 nudge 注入条件限制在明确的 budget 决策之后
- [Risk] observability 汇总与现有事件定义重复
  - Mitigation：事件继续作为原始事实源，聚合结果只作为治理和诊断视图，不替代事件日志
- [Risk] compaction tail 过度设计
  - Mitigation：优先复用现有 `recent_turn_event_tail`；只有证明不足时才新增显式快照结构
- [Trade-off] `submit_prompt` 的单次调用可能触发更多后台步骤
  - Mitigation：通过 spec 明确这是新行为，并在 SSE/指标层暴露清晰的 turn 进展

## Migration Plan

1. 在 `session-runtime` 中把 token budget 判断接到 `run_turn`
2. 增加 auto-continue nudge 注入和 continuation 上限控制
3. 把 prompt metrics / compaction 命中 / turn 耗时汇总成稳定结构
4. 根据实现结果决定是复用现有 tail 语义，还是补显式快照结构
5. 回写 `application-use-cases` 与 `session-runtime` delta specs

回滚策略：

- 如 auto-continue 行为不稳定，可先关闭 continue 判定，让 `run_turn` 回退到当前“单次调用内多 step、但不自动续写”的状态
- observability 汇总可单独保留，不影响 turn 主路径

## Open Questions

- turn 级 observability 汇总最终应暴露到哪个稳定类型：`session-runtime` 内部查询结果，还是 `application` 治理快照的组成部分？
- continuation 的触发阈值是否需要显式区分“输出被 max_tokens 截断”和“输出较短但预算充足”两种情况？
