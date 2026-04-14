## Requirements

### Requirement: Turn Chain Loop 支持 Auto-continue

session-runtime 的 turn runner SHALL 支持在单次 submit 中执行多轮 LLM 调用。当 LLM 输出后 token budget 尚有余量且输出内容较短时，系统 SHALL 自动注入 continue nudge 消息并继续下一轮 LLM 调用。

#### Scenario: Token budget 有余量时自动续写

- **WHEN** turn 完成一轮 LLM 调用，且 `check_token_budget` 返回 `TokenBudgetDecision::Continue`
- **THEN** 系统注入一条 `UserMessage`（origin=AutoContinueNudge），继续下一轮 LLM 调用

#### Scenario: Token budget 耗尽时停止

- **WHEN** turn 完成一轮 LLM 调用，且 `check_token_budget` 返回非 `Continue` 决策
- **THEN** 系统广播 `TurnDone` 事件并结束 turn

#### Scenario: 达到最大续写次数

- **WHEN** continuation_count 达到 max_continuations 配置上限
- **THEN** 系统停止 auto-continue 并结束 turn

---

### Requirement: Token Budget 管理

session-runtime SHALL 在 turn 执行期间追踪 token 使用量，并基于配置的 budget 做出 continue/stop 决策。

#### Scenario: 首次启用 budget

- **WHEN** submit_prompt 携带 token_budget 参数
- **THEN** session state 记录 total_budget、used_tokens（初始为 0）、continuation_count（初始为 0）

#### Scenario: 每轮更新 token 使用量

- **WHEN** 一轮 LLM 调用完成后
- **THEN** 系统将 estimated_tokens_used 累加到 session state 的 used_tokens

---

### Requirement: Compaction Tail 快照

session-runtime SHALL 在 turn 执行期间维护一个 compaction tail 快照，记录最近 N 轮的关键事件，用于 auto-compaction 时保留上下文。

#### Scenario: 记录 compaction tail

- **WHEN** turn 期间产生 `should_record_compaction_tail_event` 返回 true 的事件
- **THEN** 系统将该事件的 stored 版本追加到 live_tail 列表

#### Scenario: 使用 seed 初始化 tail

- **WHEN** turn 开始时
- **THEN** 系统从 session state 的 recent_stored_events 中提取 seed，构造 `CompactionTailSnapshot`

---

### Requirement: Observability 事件收集

session-runtime SHALL 在 turn 执行期间收集 prompt metrics 并报告给 observability 系统。

#### Scenario: 收集 cache reuse 指标

- **WHEN** turn 期间收到 `PromptMetrics` 事件且 prompt_cache_reuse_hits > 0
- **THEN** 系统将 reuse hits 记录到 observability

#### Scenario: 收集 turn 执行耗时

- **WHEN** turn 完成（无论成功或失败）
- **THEN** 系统记录 turn 耗时和成功状态

---

### Requirement: turn loop SHALL 记录显式 transition 原因

`session-runtime` 的 turn loop MUST 为每一次“继续下一轮”的动作记录显式 transition 原因，而不是仅依赖分散的局部计数器或隐式分支。该 transition 原因 SHALL 由 `session-runtime/turn` 持有，并驱动后续 request 重新组装、LLM 重试或 budget 续写。

#### Scenario: tool 结果驱动下一轮

- **WHEN** 一轮 LLM 输出包含工具调用，且 tool cycle 成功完成
- **THEN** turn loop 记录一次显式 transition，表示“工具结果已追加，进入下一轮”
- **AND** 下一轮 prompt 重新组装 SHALL 以该 transition 为当前 continue 原因

#### Scenario: reactive compact 驱动下一轮

- **WHEN** 一轮 LLM 调用因 prompt-too-long 被恢复为 reactive compact 成功路径
- **THEN** turn loop 记录一次显式 transition，表示“压缩恢复后重新尝试”
- **AND** 系统 SHALL 在不落入普通完成路径的前提下重新组装请求

#### Scenario: budget 允许 auto-continue

- **WHEN** turn loop 在一次 assistant 输出后判断 budget 允许继续
- **THEN** turn loop 记录一次显式 transition，表示“budget 允许续写”
- **AND** 系统注入对应的 continue nudge 后进入下一轮

---

### Requirement: turn loop SHALL 从输出截断处继续恢复

当 LLM 以 `max_tokens` 或等价输出上限原因结束当前 assistant 输出时，`session-runtime` 的 turn loop MUST 把该场景视为可恢复的 loop 分支，而不是只记录 warning 并直接结束本次输出。

#### Scenario: 无 tool call 的截断输出触发恢复

- **WHEN** 一轮 LLM 输出以输出上限结束，且当前 assistant 输出不包含 tool calls
- **THEN** 系统注入一条专用的 synthetic continuation prompt
- **AND** turn loop SHALL 在同一次 turn 内继续下一轮 LLM 调用

#### Scenario: 达到恢复上限后停止

- **WHEN** 输出截断恢复次数达到配置上限
- **THEN** turn loop SHALL 停止继续恢复
- **AND** 当前 turn 以明确 stop cause 结束

#### Scenario: 可恢复中的中间截断不立即变成最终失败

- **WHEN** 一次 `max_tokens` 截断仍满足自动恢复条件且尚未达到恢复上限
- **THEN** 系统 SHALL 注入 continuation prompt 并继续 turn loop
- **AND** 该中间截断 SHALL NOT 被当作最终失败立即释放

#### Scenario: 带 tool call 的截断输出不自动恢复

- **WHEN** assistant 输出以输出上限结束且包含 tool calls
- **THEN** 系统 SHALL NOT 自动注入 continuation prompt
- **AND** 该场景 SHALL 按更保守的结束或错误路径处理

---

### Requirement: turn loop SHALL 在流式 LLM 阶段组装可执行工具候选并提前调度安全工具

`session-runtime` 的 turn loop MUST 能在接收 LLM `ToolCallDelta` 的过程中先组装可执行工具候选，并对“参数已闭合、调用 identity 稳定且能力声明为 concurrency-safe”的工具调用提前开始执行，而不是始终等待完整 `LlmOutput` 返回。

#### Scenario: 只读工具在流式阶段提前执行

- **WHEN** LLM 流式产出的一组 `ToolCallDelta` 已组装成一个参数已闭合、identity 稳定且 `concurrency_safe` 的工具调用候选
- **THEN** 系统 SHALL 在 assistant 完整输出结束前就开始该工具执行
- **AND** 该执行 SHALL 仍然属于当前 step

#### Scenario: 副作用工具保持保守调度

- **WHEN** LLM 流式产出的工具调用不满足 `concurrency_safe`、参数尚未闭合或 identity 尚未稳定
- **THEN** 系统 SHALL 不提前执行该调用
- **AND** 它 SHALL 回退到完整 assistant 输出后的常规 tool cycle

#### Scenario: 候选在后续流式阶段失效时丢弃提前执行结果

- **WHEN** 一个已提前执行的工具候选在后续流式阶段被证明不再与 assistant 最终工具计划一致
- **THEN** 系统 SHALL 丢弃该候选的提前执行结果
- **AND** 该结果 SHALL NOT 进入 durable tool 事实流

#### Scenario: durable 顺序保持 assistant 先于工具事实

- **WHEN** 某个工具调用在流式阶段提前开始甚至提前完成
- **THEN** durable 事件写入顺序仍 SHALL 保持 assistant 定稿在前
- **AND** tool call / tool result 事实 SHALL 在该 assistant 定稿之后有序落盘
