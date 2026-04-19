## Purpose

定义评测运行器，通过 server HTTP 控制面与本地 JSONL 数据面编排评测任务的执行、结果收集与基线对比。

## ADDED Requirements

### Requirement: 评测运行器 SHALL 作为独立 binary 执行

系统 MUST 提供 `astrcode-eval-runner` 独立 binary，通过命令行参数控制评测执行。

#### Scenario: 执行指定任务集

- **WHEN** 运行 `astrcode-eval-runner --server-url http://localhost:3000 --session-storage-root ./.astrcode-eval-state --task-set eval-tasks/task-set.yaml`
- **THEN** 运行器 MUST 加载任务集内所有任务定义
- **AND** 依次或并行执行每个任务
- **AND** 输出评测结果到 stdout 或指定文件

#### Scenario: 指定 server 不可达

- **WHEN** 运行器无法连接到指定的 server URL
- **THEN** 运行器 MUST 在启动阶段报错并退出
- **AND** 不执行任何任务

### Requirement: 运行器 SHALL 通过 server HTTP API 驱动任务执行

每个评测任务的执行 MUST 通过标准 server API 完成控制面操作，并通过共享 session 存储中的 JSONL 完成 durable 结果读取。

#### Scenario: 单任务执行流程

- **WHEN** 运行器开始执行一个评测任务
- **THEN** MUST 按序执行：准备工作区 → 创建 session → 提交 turn → 等待完成 → 读取 trace → 诊断 → 评分 → 收集结果
- **AND** 每一步的失败 MUST 记录到结果中，不中断其他任务的执行

#### Scenario: 创建 session 指向隔离工作区

- **WHEN** 运行器创建评测用 session
- **THEN** session 的 `working_dir` MUST 指向该任务的隔离工作区目录
- **AND** 不同任务的 session MUST 使用不同的工作区

#### Scenario: 等待 turn 完成

- **WHEN** 运行器提交 turn 后等待完成
- **THEN** MUST 通过轮询共享 session 存储中的 JSONL 文件检测 `TurnDone` durable 事件
- **AND** MUST 设置超时（可配置，默认 5 分钟），超时后标记任务为 `timeout`

#### Scenario: 控制面可达但数据面不可达

- **WHEN** 运行器可以连接 `server-url`，但无法访问对应的 `session_storage_root`
- **THEN** 运行器 MUST 在启动阶段或首个任务前明确失败
- **AND** 错误信息 MUST 指出控制面 / 数据面不一致，而不是静默退化为不稳定的等待策略

### Requirement: 运行器 SHALL 支持工作区隔离与清理

评测任务的工作区 MUST 与其他任务隔离，并在评测结束后清理。

#### Scenario: 从 fixture 准备隔离工作区

- **WHEN** 任务定义指定了 `workspace.setup` 目录
- **THEN** 运行器 MUST 将 fixture 目录完整复制到 `/tmp/astrcode-eval-{run_id}/{task_id}/`
- **AND** 复制后验证目标目录存在且文件完整

#### Scenario: 评测结束后清理

- **WHEN** 所有任务执行完毕且结果已持久化
- **THEN** 运行器 MUST 删除所有隔离工作区目录
- **AND** 当使用 `--keep-workspace` 参数时，SHALL 保留工作区并输出路径

### Requirement: 运行器 SHALL 支持基线对比

评测结果 MUST 支持与历史基线进行指标对比。

#### Scenario: 与指定基线对比

- **WHEN** 运行 `astrcode-eval-runner --baseline results/baseline-2026-04-15.json`
- **THEN** 运行器 MUST 在当前评测结果中附加与基线的 diff
- **AND** diff MUST 包含各任务的分数变化、指标变化（工具调用数、token 消耗、耗时）
- **AND** 分数下降超过阈值时 MUST 输出警告

#### Scenario: 基线文件不存在

- **WHEN** 指定的基线文件路径不存在
- **THEN** 运行器 MUST 发出警告但继续执行
- **AND** 评测结果中不包含对比数据

### Requirement: 运行器 SHALL 输出结构化评测报告

评测完成后 MUST 输出结构化的 JSON 报告。

#### Scenario: 生成评测报告

- **WHEN** 所有评测任务执行完毕
- **THEN** 报告 MUST 包含：运行元数据（commit SHA、时间戳、任务集名称）、各任务结果（状态、分数、指标、失败诊断）、汇总统计（通过率、平均分数、各维度平均指标）
- **AND** 报告 MUST 可序列化为 JSON 并持久化到文件

#### Scenario: 报告中包含诊断信息

- **WHEN** 某个任务被失败诊断器检测到问题
- **THEN** 报告中该任务的结果 MUST 包含完整的 `DiagnosisReport`
- **AND** 诊断信息与评测结果关联，支持后续分析

### Requirement: 运行器 SHALL 支持并行任务执行

运行器 MUST 支持同时执行多个评测任务以提高效率。

#### Scenario: 配置并发度

- **WHEN** 运行 `astrcode-eval-runner --concurrency 4`
- **THEN** 运行器 MUST 最多同时执行 4 个评测任务
- **AND** 每个任务使用独立的 session，互不干扰

#### Scenario: 并行任务中某个失败

- **WHEN** 并行执行中某个任务失败
- **THEN** 运行器 MUST 记录该任务的失败结果
- **AND** 不影响其他正在执行的任务
- **AND** 所有任务执行完毕后汇总全部结果
