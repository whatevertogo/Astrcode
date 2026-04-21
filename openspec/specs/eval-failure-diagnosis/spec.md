# eval-failure-diagnosis Specification

## Purpose

定义基于规则引擎的失败模式自动诊断系统，从 TurnTrace 中检测已知失败模式并生成结构化诊断报告。

## Requirements

### Requirement: 诊断器 SHALL 使用可扩展的 trait 接口

系统 MUST 定义 `FailurePatternDetector` trait，所有具体检测器实现该 trait，支持注册和组合使用。

#### Scenario: 注册并执行多个检测器

- **WHEN** 诊断引擎初始化时注册了 N 个检测器
- **THEN** 对一个 `TurnTrace` 执行诊断时 MUST 依次调用所有检测器
- **AND** 汇总所有检测器的输出为完整诊断报告

#### Scenario: 检测器输出结构化诊断实例

- **WHEN** 某个检测器在 trace 中发现了匹配的失败模式
- **THEN** MUST 输出 `FailureInstance`，包含：模式名称、严重级别、置信度、涉及的 `storage_seq` 范围、结构化的上下文数据和人类可读的描述
- **AND** `storage_seq` 范围 MUST 允许精确回溯到原始 JSONL 事件

### Requirement: 工具循环检测器 SHALL 识别重复工具调用

系统 MUST 提供 `ToolLoopDetector`，检测同一工具被重复调用且参数相似的情况。

#### Scenario: 检测到工具循环

- **WHEN** 一个 turn 内同一工具名称连续出现 ≥ 3 次
- **AND** 相邻调用的参数相似度 > 配置的阈值
- **THEN** 检测器 MUST 输出一个 `FailureInstance`，severity 为 `high`
- **AND** 上下文 MUST 包含重复调用的 `tool_call_id` 列表和参数对比

#### Scenario: 同名工具但参数差异大

- **WHEN** 同一工具名称连续出现 ≥ 3 次
- **AND** 参数之间无显著相似性（如对不同文件的操作）
- **THEN** 检测器 MUST NOT 报告为循环
- **AND** 该情况属于正常的多文件操作

### Requirement: 级联失败检测器 SHALL 识别连续工具失败

系统 MUST 提供 `CascadeFailureDetector`，检测连续多次工具调用失败的情况。

#### Scenario: 连续工具调用失败

- **WHEN** 一个 turn 内连续 ≥ 2 次 `ToolResult` 的 `success` 为 false
- **THEN** 检测器 MUST 输出 `FailureInstance`，severity 为 `high`
- **AND** 上下文 MUST 包含失败工具序列和各自的错误信息

#### Scenario: 工具失败后重试成功

- **WHEN** 某个工具调用失败后，后续调用同一工具成功
- **THEN** 检测器 MUST NOT 报告为级联失败
- **AND** 这是正常的重试恢复行为

### Requirement: Compact 信息丢失检测器 SHALL 识别压缩后的功能退化

系统 MUST 提供 `CompactInfoLossDetector`，检测上下文压缩后紧接着出现工具调用失败的情况。

#### Scenario: compact 后工具调用失败

- **WHEN** turn 内发生了 `CompactApplied` 事件
- **AND** compact 之后出现了 `ToolResult` 失败，且失败原因暗示信息丢失（如"文件不存在"而文件实际存在）
- **THEN** 检测器 MUST 输出 `FailureInstance`，severity 为 `medium`
- **AND** 上下文 MUST 包含 compact 的 token 变化（pre/post）和后续失败的工具调用详情

#### Scenario: compact 后正常继续

- **WHEN** turn 内发生了 `CompactApplied` 事件，但后续所有工具调用成功
- **THEN** 检测器 MUST NOT 报告
- **AND** 这是健康的 compact 行为

### Requirement: 子 Agent 预算超支检测器 SHALL 识别子运行超限

系统 MUST 提供 `SubRunBudgetDetector`，检测子 Agent 执行超过预设步数限制的情况。

#### Scenario: 子 Agent 超过步数限制

- **WHEN** `SubRunFinished` 的 `step_count` 超过 `ResolvedExecutionLimitsSnapshot` 中的步数限制
- **THEN** 检测器 MUST 输出 `FailureInstance`，severity 为 `medium`
- **AND** 上下文 MUST 包含实际步数与限制的对比

#### Scenario: 子 Agent 在限制内完成

- **WHEN** `SubRunFinished` 的 `step_count` 未超过限制
- **THEN** 检测器 MUST NOT 报告

### Requirement: 空 turn 检测器 SHALL 识别无效 turn

系统 MUST 提供 `EmptyTurnDetector`，检测 turn 结束但未产出任何有意义内容的情况。

#### Scenario: turn 无工具调用且输出为空

- **WHEN** turn 完成（有 `TurnDone` 事件）
- **AND** 无任何 `ToolCall` 事件
- **AND** `AssistantFinal` 的 `content` 长度 < 配置的最小阈值
- **THEN** 检测器 MUST 输出 `FailureInstance`，severity 为 `medium`

#### Scenario: turn 仅有文本输出

- **WHEN** turn 无工具调用但 `AssistantFinal` 包含有意义的回复
- **THEN** 检测器 MUST NOT 报告
- **AND** 这是正常的纯对话行为

### Requirement: 诊断报告 SHALL 为结构化可持久化格式

系统 MUST 将诊断结果输出为可序列化的结构化报告。

#### Scenario: 生成诊断报告

- **WHEN** 对一组 `TurnTrace` 执行完整诊断
- **THEN** 输出 `DiagnosisReport` MUST 包含：session 元数据、turn 级诊断结果列表、汇总统计（各模式出现次数、严重级别分布）
- **AND** 报告 MUST 可序列化为 JSON 格式

#### Scenario: 诊断报告支持可复现回溯

- **WHEN** 诊断报告中的某个 `FailureInstance` 引用了 `storage_seq` 范围 [100, 108]
- **THEN** 读者 MUST 能从原始 JSONL 文件中精确定位这些事件
- **AND** 复现路径为：打开 JSONL → 定位 seq 范围 → 读取对应 `StorageEvent`
