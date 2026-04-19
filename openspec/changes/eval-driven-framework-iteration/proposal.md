## Why

Astrcode 目前拥有完善的事件溯源架构（`StorageEvent` JSONL 日志）和运行时 observability 管线，但缺乏一套**系统化的评测体系**来驱动框架自身的质量迭代。当前状态是：

- 丰富的运行时事件数据（工具调用链、token 指标、compaction 效果、子 Agent 生命周期）已持久化但未被系统化分析
- 框架改动（prompt 策略调整、compaction 算法变更、工具行为修改）缺乏量化回归手段，依赖人工试用感知
- Agent 失败模式（工具循环、上下文丢失、规划偏差）无自动诊断能力，需要人工逐条翻阅 JSONL

这意味着每次框架迭代都是"改了就跑跑看"，无法做到**度量驱动的精准优化**。现在启动评测体系是因为事件层和 observability 层已足够成熟（`PromptMetrics`、`CompactApplied`、`AgentCollaborationFact` 等已稳定），具备了在零运行时改动的前提下启动离线评测的条件。

## What Changes

- **引入评测 trace 模型**：定义从 `StorageEvent` JSONL 中提取的结构化评测数据模型（`EvalTurnTrace`），覆盖单 turn 内的工具调用链、token 消耗、compaction 事件、错误序列与时间线
- **引入评测任务规范**：定义结构化的评测任务描述格式（YAML），包含任务输入、工作区快照、期望行为约束（工具序列、文件变更、步数上限）和评分规则
- **引入失败模式诊断器**：基于事件模式的规则引擎，自动从 turn trace 中检测已知失败模式（工具循环、级联失败、上下文丢失、子 Agent 预算超支），生成结构化诊断报告
- **引入评测运行器**：通过现有 server HTTP API 编排评测任务执行的独立 binary，支持并行运行、工作区隔离与结果汇总
- **引入评测回归对比**：存储评测基线结果，支持版本间指标 diff，用于 CI 中自动检测质量退化

## Capabilities

### New Capabilities

- `eval-trace-model`: 从 StorageEvent JSONL 提取的结构化评测 trace 数据模型，包括 TurnTrace、ToolCallRecord、FailurePattern 等核心类型，以及 JSONL → trace 的提取器
- `eval-task-spec`: 结构化评测任务规范定义，包括任务描述格式（YAML）、工作区快照管理、期望行为约束与评分规则
- `eval-failure-diagnosis`: 基于规则引擎的失败模式自动诊断系统，从 turn trace 中检测工具循环、级联失败、compaction 信息丢失、子 Agent 预算超支等模式，输出结构化诊断报告
- `eval-runner`: 评测任务编排与执行运行器，通过 server HTTP API 驱动任务执行，支持并行运行、工作区隔离、结果收集与基线对比

### Modified Capabilities

- `runtime-observability-pipeline`: 需扩展以支持评测场景下的指标导出（将 live metrics 写入评测结果而非仅推送到 SSE/frontend），确保评测运行时的 observability 数据可被评测运行器收集
- `agent-tool-evaluation`: 现有的 agent 协作评估记录应作为评测 trace 的输入源之一，需要在评测 trace 模型中建立与 collaboration facts 的关联

## Impact

- **新增 crate**：`crates/eval` — 独立的评测 crate，包含 trace 模型、任务规范、诊断器和运行器。仅依赖 `core`（复用 `StorageEvent` 等类型）和 `protocol`（复用 HTTP DTO），不侵入现有运行时路径
- **无运行时改动**：Phase 1 完全基于离线 JSONL 分析，不需要修改 `session-runtime`、`application` 或 `server`
- **CI 集成**：评测运行器作为独立 binary 在 CI 中调用，不影响现有构建流程
- **前端影响**：Phase 1 无前端改动。后续可在现有 Debug Workbench 基础上扩展评测视图（不在本次 scope 内）

## Non-Goals

- **不在本次构建 LLM-as-Judge**：语义级别的评测（如"输出是否正确"）需要额外 LLM 调用，留作后续迭代
- **不在本次构建前端评测仪表板**：评测结果以 JSON/Markdown 报告输出，前端可视化留作后续迭代
- **不在本次引入 container 化隔离**：工作区隔离通过文件系统 copy 实现，不需要 Docker/容器
- **不在本次修改现有 StorageEvent 格式**：完全复用现有事件类型，不新增运行时事件
- **不在本次做跨模型评测**：评测聚焦于框架行为（工具调用效率、token 消耗、失败模式），不做模型能力横向对比
