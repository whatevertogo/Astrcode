## Requirements

### Requirement: Turn Chain Loop 支持输出截断恢复

session-runtime 的 turn runner SHALL 支持在单次 submit 中执行多轮 LLM 调用，并在 assistant 输出因 `max_tokens` 截断且无 tool calls 时自动注入 continuation prompt，在同一 turn 内继续下一轮 LLM 调用。

#### Scenario: 不需要续写时自然结束

- **WHEN** turn 完成一轮 LLM 调用，且 `decide_output_continuation` 返回 `NotNeeded`
- **THEN** 系统广播 `TurnDone` 事件（reason="completed"）并结束 turn

---

### Requirement: Token 使用量管理

session-runtime SHALL 在 turn 执行期间追踪 provider 报告的 token 使用量，并将其用于 observability 汇总。

#### Scenario: 每轮更新 token 使用量

- **WHEN** 一轮 LLM 调用完成后
- **THEN** 系统通过 `TokenUsageTracker::record_usage` 记录 usage，累加 `cache_read_input_tokens` 和 `cache_creation_input_tokens`

---

### Requirement: Micro Compact State 管理

session-runtime SHALL 在 turn 执行期间维护 `MicroCompactState` 和 `FileAccessTracker`，用于跟踪最近的助手活动和文件访问记录，支持微压缩和文件恢复。

#### Scenario: 使用 seed 初始化 micro compact state

- **WHEN** turn 开始时（`TurnExecutionContext::new`）
- **THEN** 系统通过 `MicroCompactState::seed_from_messages` 从已有消息构造初始状态，使用 `ContextWindowSettings.micro_compact_config()` 配置

#### Scenario: 使用 seed 初始化 file access tracker

- **WHEN** turn 开始时
- **THEN** 系统通过 `FileAccessTracker::seed_from_messages` 从已有消息中提取文件访问记录，使用 `max_tracked_files` 和 `working_dir` 配置

#### Scenario: 工具结果记录到追踪器

- **WHEN** 一轮工具执行完成后
- **THEN** 系统将每个工具结果通过 `FileAccessTracker::record_tool_result` 和 `MicroCompactState::record_tool_result` 记录

---

### Requirement: Observability 事件收集

session-runtime SHALL 在 turn 执行期间收集 prompt metrics 并报告给 observability 系统。

#### Scenario: 收集 cache reuse 指标

- **WHEN** turn 期间 step 组装 prompt 时（`request` 模块），通过 `PromptBuildCacheMetrics` 收集 reuse_hits、reuse_misses、unchanged_layers
- **THEN** 系统生成 `PromptMetrics` 事件并追加到 events 列表

#### Scenario: 收集 turn 执行耗时和汇总

- **WHEN** turn 完成（无论成功或失败）
- **THEN** 系统生成 `TurnSummary`，包含 `wall_duration`、`step_count`、`total_tokens_used`、`cache_read_input_tokens`、`cache_creation_input_tokens`、`auto_compaction_count`、`reactive_compact_count`、`max_output_continuation_count`、`streaming_tool_*` 系列指标和 `collaboration` 汇总

#### Scenario: 收集 Provider 使用量回填

- **WHEN** LLM 返回 `LlmUsage` 后
- **THEN** 系统通过 `apply_prompt_metrics_usage` 将 provider 报告的 input/output tokens 和 cache 指标回填到已有的 `PromptMetrics` 事件中

---

### Requirement: turn loop SHALL 记录显式 transition 原因

`session-runtime` 的 turn loop MUST 为每一次"继续下一轮"的动作记录显式 transition 原因，而不是仅依赖分散的局部计数器或隐式分支。该 transition 原因 SHALL 由 `TurnLoopTransition` 枚举持有，并驱动后续 request 重新组装、LLM 重试或 budget 续写。

#### Scenario: tool 结果驱动下一轮

- **WHEN** 一轮 LLM 输出包含工具调用，且 tool cycle 成功完成
- **THEN** turn loop 记录一次 `TurnLoopTransition::ToolCycleCompleted`
- **AND** `execution.step_index` 递增，下一轮 prompt 重新组装 SHALL 以该 transition 为当前 continue 原因

#### Scenario: reactive compact 驱动下一轮

- **WHEN** 一轮 LLM 调用因 prompt-too-long 被恢复为 reactive compact 成功路径
- **THEN** turn loop 记录一次 `TurnLoopTransition::ReactiveCompactRecovered`
- **AND** 系统 SHALL 在不落入普通完成路径的前提下重新组装请求

#### Scenario: 输出截断恢复驱动下一轮

- **WHEN** `decide_output_continuation` 返回 Continue
- **THEN** turn loop 记录一次 `TurnLoopTransition::OutputContinuationRequested`
- **AND** 系统注入 `OUTPUT_CONTINUATION_PROMPT` 用户消息后进入下一轮

---

### Requirement: turn loop SHALL 从输出截断处继续恢复

当 LLM 以 `max_tokens` 或等价输出上限原因结束当前 assistant 输出时，`session-runtime` 的 turn loop MUST 把该场景视为可恢复的 loop 分支，而不是只记录 warning 并直接结束本次输出。

#### Scenario: 无 tool call 的截断输出触发恢复

- **WHEN** 一轮 LLM 输出以 `LlmFinishReason::MaxTokens` 结束，且当前 assistant 输出不包含 tool calls
- **THEN** 系统注入一条 `OUTPUT_CONTINUATION_PROMPT` 用户消息（origin=`ContinuationPrompt`）
- **AND** turn loop SHALL 在同一次 turn 内继续下一轮 LLM 调用

#### Scenario: 达到恢复上限后停止

- **WHEN** `max_output_continuation_count` 达到 `ResolvedRuntimeConfig.max_output_continuation_attempts` 配置上限
- **THEN** turn loop SHALL 停止继续恢复
- **AND** 当前 turn 以 `TurnStopCause::MaxOutputContinuationLimitReached` 结束

#### Scenario: 可恢复中的中间截断不立即变成最终失败

- **WHEN** 一次 `max_tokens` 截断仍满足自动恢复条件且尚未达到恢复上限
- **THEN** 系统 SHALL 注入 continuation prompt 并继续 turn loop
- **AND** 该中间截断 SHALL NOT 被当作最终失败立即释放

#### Scenario: 带 tool call 的截断输出不自动恢复

- **WHEN** assistant 输出以 `LlmFinishReason::MaxTokens` 结束且包含 tool calls
- **THEN** 系统 SHALL NOT 自动注入 continuation prompt
- **AND** 该场景 SHALL 按常规 tool cycle 路径处理

---

### Requirement: turn loop SHALL 在流式 LLM 阶段组装可执行工具候选并提前调度安全工具

`session-runtime` 的 turn loop MUST 能在接收 LLM `ToolCallDelta` 的过程中先组装可执行工具候选，并对"参数已闭合、调用 identity 稳定且能力声明为 concurrency-safe"的工具调用提前开始执行，而不是始终等待完整 `LlmOutput` 返回。

#### Scenario: 只读工具在流式阶段提前执行

- **WHEN** LLM 流式产出的 `StreamedToolCallDelta` 通过 `StreamingToolAssembler` 组装成一个参数已闭合（JSON 完整闭合）、identity 稳定（id 和 name 不再变化）且 `concurrency_safe` 的工具调用候选
- **THEN** `StreamingToolLauncher` SHALL 通过 `execute_buffered_tool_call` 在 assistant 完整输出结束前就开始该工具执行
- **AND** 该执行 SHALL 仍然属于当前 step

#### Scenario: 副作用工具保持保守调度

- **WHEN** LLM 流式产出的工具调用不满足 `concurrency_safe`、参数尚未闭合（JSON 未完成）或 identity 尚未稳定
- **THEN** 系统 SHALL 不提前执行该调用
- **AND** 它 SHALL 回退到完整 assistant 输出后的常规 tool cycle

#### Scenario: 候选在后续流式阶段失效时丢弃提前执行结果

- **WHEN** 一个已提前执行的工具候选在 `StreamingToolReconciler::reconcile` 中被证明不再与 assistant 最终工具计划一致（请求不匹配或 join 失败）
- **THEN** 系统 SHALL abort 该候选的提前执行并记录 `StreamingToolFallbackReason`
- **AND** 该结果 SHALL NOT 进入 durable tool 事实流

#### Scenario: durable 顺序保持 assistant 先于工具事实

- **WHEN** 某个工具调用在流式阶段提前开始甚至提前完成
- **THEN** 当使用 streaming path 时，`ToolEventEmissionMode::Buffered` 模式 SHALL 先写入 assistant 定稿事件
- **AND** tool call / tool result 事实 SHALL 在该 assistant 定稿之后通过 `merge_buffered_and_remaining_tool_results` 有序落盘

#### Scenario: 流式工具统计追踪

- **WHEN** 流式工具执行完成后
- **THEN** 系统 SHALL 在 `TurnSummary` 中记录 `streaming_tool_launch_count`、`streaming_tool_match_count`、`streaming_tool_fallback_count`、`streaming_tool_discard_count`、`streaming_tool_overlap_ms` 指标

---

### Requirement: turn request assembly SHALL 应用 persisted tool-result references

`session-runtime` 的 turn request assembly MUST 在组装 LLM 请求前，对同一 API-level user tool-result 批次应用 persisted reference replacement，而不是把所有原始 tool result 原样塞进最终 prompt。

#### Scenario: over-budget tool-result batch 使用 persisted reference replacement

- **WHEN** `apply_tool_result_budget` 检测到同一批 trailing tool result 总字节超过 `aggregate_budget_bytes`
- **THEN** 系统 SHALL 将最大的 fresh tool result 按大小降序依次替换为 `<persisted-output>` 引用文本（通过 `persist_tool_result` 持久化到磁盘）
- **AND** 最终 LLM 请求 SHALL 使用 replacement 后的消息序列

#### Scenario: 已替换过的结果在后续轮次中重放相同 replacement

- **WHEN** 某个 `tool_call_id` 之前已经应用过 persisted reference replacement（存在于 `ToolResultReplacementState.replacements` 中）
- **THEN** 后续 `apply_tool_result_budget` SHALL 重放完全相同的 replacement 文本
- **AND** SHALL NOT 重新生成语义等价但字节不同的引用文本

#### Scenario: 已看过但未替换的结果不得在后续轮次补替换

- **WHEN** 某个 `tool_call_id` 之前已经进入 prompt 且未被替换（被 `ToolResultReplacementState.frozen` 标记）
- **THEN** 后续 request assembly SHALL 保持其未替换状态
- **AND** SHALL NOT 在后续轮次中突然把它替换为 persisted reference

---

### Requirement: Turn 执行结果汇总

session-runtime SHALL 在每次 turn 执行结束后生成不可变的 `TurnSummary`，供治理/诊断读取路径消费。

#### Scenario: TurnSummary 包含执行指标

- **WHEN** turn 执行完成（`run_turn` 返回 `TurnRunResult`）
- **THEN** `TurnSummary` 包含：`finish_reason`（`TurnFinishReason`）、`stop_cause`（`TurnStopCause`）、`last_transition`、`wall_duration`、`step_count`、`total_tokens_used`、cache 指标、压缩次数、tool-result replacement 指标、streaming tool 指标、`collaboration` 汇总

#### Scenario: TurnFinishReason 映射

- **WHEN** `TurnStopCause` 转换为 `TurnFinishReason`
- **THEN** Completed / MaxOutputContinuationLimitReached → NaturalEnd；Cancelled → Cancelled；Error → Error；StepLimitExceeded → StepLimitExceeded

#### Scenario: Collaboration 汇总聚合

- **WHEN** turn 结束时
- **THEN** 系统从 session 事件流中过滤当前 turn 的 `AgentCollaborationFact`，通过 `TurnCollaborationSummary::from_facts` 聚合 spawn/send/observe/close/delivery 计数、rejected/failed/reused 计数、以及 delivery latency 统计
