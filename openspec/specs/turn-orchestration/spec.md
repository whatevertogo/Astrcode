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
