## ADDED Requirements

### Requirement: Compaction Tail 快照
session-runtime SHALL 在 turn 执行期间维护一个 compaction tail 快照，记录最近 N 轮的关键事件，用于 auto-compaction 时保留上下文。

#### Scenario: 记录 compaction tail
- **WHEN** turn 期间产生 `should_record_compaction_tail_event` 返回 true 的事件
- **THEN** 系统将该事件的 stored 版本追加到 live_tail 列表

#### Scenario: 使用 seed 初始化 tail
- **WHEN** turn 开始时
- **THEN** 系统从 session state 的 recent_stored_events 中提取 seed，构造 `CompactionTailSnapshot`

### Requirement: Observability 事件收集
session-runtime SHALL 在 turn 执行期间收集 prompt metrics 并报告给 observability 系统。

#### Scenario: 收集 cache reuse 指标
- **WHEN** turn 期间收到 `PromptMetrics` 事件且 prompt_cache_reuse_hits > 0
- **THEN** 系统将 reuse hits 记录到 observability

#### Scenario: 收集 turn 执行耗时
- **WHEN** turn 完成（无论成功或失败）
- **THEN** 系统记录 turn 耗时和成功状态
