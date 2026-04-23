# eval-trace-model Specification

## Purpose

定义评测 trace 数据模型，将 StorageEvent JSONL 事件流转化为结构化的 session / turn 级评测数据。

## Requirements

### Requirement: TurnTrace SHALL 作为评测数据的核心单元

系统 MUST 定义 `TurnTrace` 结构体，包含单个 turn 内的所有评测相关信息：用户输入、工具调用序列、助手输出、prompt 指标、compaction 事件、错误事件、协作事实摘要和时间线。

#### Scenario: 从完整的 turn 事件序列提取 TurnTrace

- **WHEN** 提取器接收到一个 turn 的所有 `StorageEvent`（从 `UserMessage` 到 `TurnDone`）
- **THEN** 输出 `TurnTrace` MUST 包含用户输入内容、按时间序排列的工具调用记录、助手最终输出、所有 `PromptMetrics` 快照和所有 `CompactApplied` 事件
- **AND** 每个工具调用记录 MUST 包含工具名称、参数、输出、成功状态和持续时间（`duration_ms`）

#### Scenario: 处理不完整 turn（无 TurnDone 事件）

- **WHEN** 提取器遇到一组事件没有 `TurnDone` 结束标记（如 session 崩溃）
- **THEN** 提取器 MUST 仍然输出 `TurnTrace`
- **AND** 该 `TurnTrace` MUST 标记为 `incomplete: true`

#### Scenario: turn 内包含子 Agent 执行

- **WHEN** turn 内存在 `SubRunStarted` 和 `SubRunFinished` 事件
- **THEN** `TurnTrace` MUST 包含 `SubRunTrace`，记录子 Agent 的 step_count、estimated_tokens、执行结果、持续时间和 `resolved_limits`
- **AND** 子 Agent 的 `child_session_id` MUST 被记录，支持后续递归提取子 session 的 trace

#### Scenario: turn 内包含协作评估事实

- **WHEN** turn 内存在 `AgentCollaborationFact` 事件
- **THEN** `TurnTrace` MUST 记录协作事实摘要，并在存在 `sub_run_id` 时与对应 `SubRunTrace` 建立关联
- **AND** 该协作摘要 SHALL 可用于后续 agent delegation 效果评估

### Requirement: TraceExtractor SHALL 从 JSONL 文件提取 SessionTrace

系统 MUST 提供 `TraceExtractor`，接受 JSONL 文件路径，输出 `SessionTrace`；其中 session 级元数据与 `Vec<TurnTrace>` 必须同时可用。

#### Scenario: 从单个 session JSONL 提取所有 turn trace

- **WHEN** 对一个包含多个 turn 的 session JSONL 文件执行提取
- **THEN** 提取器 MUST 返回一个 `SessionTrace`
- **AND** 其中的 `turns` 数量 MUST 与 durable turn 数量一致
- **AND** 每个 `TurnTrace` MUST 按事件时间序构建

#### Scenario: 处理包含 SessionStart 的事件流

- **WHEN** JSONL 文件以 `SessionStart` 事件开始
- **THEN** 提取器 MUST 记录 session 元数据（session_id、working_dir、timestamp）
- **AND** `SessionStart` 不产生独立 `TurnTrace`，而是作为 `SessionTrace` 的 header

#### Scenario: 处理跨 agent 谱系事件

- **WHEN** JSONL 中的事件携带 `AgentEventContext`（非空的 agent_id、parent_turn_id、sub_run_id）
- **THEN** 提取器 MUST 在 `TurnTrace` 中保留 agent 谱系信息
- **AND** 支持 root agent 和 sub-run agent 的 trace 区分

### Requirement: ToolCallRecord SHALL 记录工具调用的完整生命周期

系统 MUST 定义 `ToolCallRecord`，从 `ToolCall` + `ToolCallDelta` + `ToolResult` 事件中构建完整的工具调用记录。

#### Scenario: 正常完成的工具调用

- **WHEN** 提取器遇到 `ToolCall` 事件，随后在同一 `tool_call_id` 上遇到 `ToolResult` 事件
- **THEN** `ToolCallRecord` MUST 包含工具名称、参数、输出、成功状态、持续时间和流式输出增量（如果有）

#### Scenario: 工具调用有流式输出

- **WHEN** 工具调用过程中产生了 `ToolCallDelta` 事件
- **THEN** `ToolCallRecord` MUST 累积流式输出增量
- **AND** 最终的 `ToolResult` 中的 `output` 为完整输出，不包含中间增量

#### Scenario: 工具调用结果被持久化引用替换

- **WHEN** 工具调用后产生了 `ToolResultReferenceApplied` 事件
- **THEN** `ToolCallRecord` MUST 记录原始输出大小（`original_bytes`）和替换后的引用
- **AND** 该信息用于评估大输出的处理效率
