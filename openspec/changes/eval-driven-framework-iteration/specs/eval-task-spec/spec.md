## Purpose

定义结构化的评测任务规范，支持可重复、可对比的 Agent 行为评测执行。

## ADDED Requirements

### Requirement: 评测任务 SHALL 使用 YAML 格式定义

系统 MUST 支持从 YAML 文件加载评测任务定义，每个文件描述一个独立的评测任务。

#### Scenario: 加载合法的任务定义文件

- **WHEN** 系统读取一个包含 `task_id`、`description`、`prompt`、`workspace`、`expected_outcome` 字段的 YAML 文件
- **THEN** 系统 MUST 成功解析为 `EvalTask` 结构体
- **AND** `task_id` MUST 为全局唯一的 kebab-case 标识符

#### Scenario: 任务定义缺少必要字段

- **WHEN** YAML 文件缺少 `task_id`、`prompt` 或 `expected_outcome` 中的任意字段
- **THEN** 系统 MUST 返回明确的校验错误，指出缺失字段
- **AND** 不执行该任务

### Requirement: 评测任务 SHALL 支持工作区快照管理

每个评测任务 MUST 能指定一个工作区快照，评测运行前从快照恢复工作区状态。

#### Scenario: 指定 fixture 目录作为工作区

- **WHEN** 任务定义中 `workspace.setup` 指向一个存在的 fixture 目录
- **THEN** 评测运行器 MUST 在执行前将该目录复制到隔离的工作区路径
- **AND** session 的 `working_dir` MUST 指向该隔离路径

#### Scenario: 任务不指定工作区

- **WHEN** 任务定义中 `workspace` 字段缺失
- **THEN** 评测运行器 MUST 使用空目录作为工作区
- **AND** 任务仍然正常执行（适用于纯对话评测场景）

#### Scenario: 评测完成后工作区清理

- **WHEN** 评测任务执行完成且结果已收集
- **THEN** 评测运行器 SHALL 清理隔离工作区目录
- **AND** 如果保留工作区有助于调试，SHALL 支持通过 `--keep-workspace` 选项跳过清理

### Requirement: 期望行为约束 SHALL 支持多维度匹配

`expected_outcome` MUST 支持从工具调用序列、文件变更、步数限制和输出内容四个维度约束期望行为。

#### Scenario: 约束期望的工具调用序列

- **WHEN** `expected_outcome.tool_pattern` 指定了 `["Read", "Edit"]`
- **THEN** 评分器 MUST 检查实际工具调用序列是否与期望模式前缀匹配
- **AND** 实际调用中包含期望模式之外的调用时，SHALL 扣分但不判定为失败

#### Scenario: 约束最大工具调用次数

- **WHEN** `expected_outcome.max_tool_calls` 指定为 5
- **THEN** 评分器 MUST 检查实际工具调用总数是否 ≤ 5
- **AND** 超过限制时该维度得分为 0

#### Scenario: 约束期望的文件变更

- **WHEN** `expected_outcome.file_changes` 指定了期望变更的文件路径和内容片段
- **THEN** 评分器 MUST 检查隔离工作区中对应文件是否包含期望内容
- **AND** 通过 `git diff --stat` 或文件内容匹配验证

#### Scenario: 约束最大 turn 数

- **WHEN** `expected_outcome.max_turns` 指定为 1
- **THEN** 评分器 MUST 检查任务是否在 1 个 turn 内完成
- **AND** 超过 turn 数限制时该维度得分为 0

### Requirement: 评分规则 SHALL 产生归一化综合分数

系统 MUST 将各维度的匹配结果综合为 0.0-1.0 的归一化分数。

#### Scenario: 所有必要维度全部满足

- **WHEN** 所有 `expected_outcome` 中的必要约束全部满足
- **THEN** 综合分数 MUST 为 1.0
- **AND** 任务状态为 `pass`

#### Scenario: 部分维度未满足

- **WHEN** 部分维度未满足（如工具调用超出预期但文件变更正确）
- **THEN** 综合分数 MUST 按各维度权重加权计算
- **AND** 任务状态为 `partial`

#### Scenario: 关键维度未满足

- **WHEN** 任务的核心约束未满足（如文件变更不正确）
- **THEN** 综合分数 MUST 为 0.0
- **AND** 任务状态为 `fail`

### Requirement: 任务集 SHALL 通过索引文件组织

系统 MUST 支持通过 `task-set.yaml` 索引文件组织多个任务为一个任务集。

#### Scenario: 加载任务集索引

- **WHEN** 系统读取 `task-set.yaml`，其中引用了多个任务文件路径
- **THEN** 系统 MUST 加载所有引用的任务定义
- **AND** 跳过不存在或格式错误的任务并发出警告，不中断整体评测
