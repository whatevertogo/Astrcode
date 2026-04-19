## 1. 项目骨架与 trace 模型

- [ ] 1.1 创建 `crates/eval` crate 骨架：初始化 `Cargo.toml`（依赖 `astrcode-core`、`serde`、`serde_json`、`chrono`）、`src/lib.rs` 和模块结构（`trace/`、`task/`、`diagnosis/`、`runner/`）。在 workspace `Cargo.toml` 中注册 members。验证：`cargo check -p astrcode-eval` 通过。
- [ ] 1.2 实现 trace 核心类型：在 `crates/eval/src/trace/mod.rs` 中定义 `TurnTrace`、`ToolCallRecord`、`SubRunTrace`、`SessionTrace`、`CompactTrace`、`PromptMetricsSnapshot`、协作事实摘要类型等结构体，覆盖 session 元数据、agent 谱系、`resolved_limits` 与 collaboration facts，均 derive `Serialize/Deserialize`。验证：单元测试确保 round-trip 序列化。
- [ ] 1.3 实现 `TraceExtractor`：在 `crates/eval/src/trace/extractor.rs` 中实现 JSONL 文件 → `SessionTrace` 的提取逻辑。逐行读取 JSONL，通过 `serde_json::from_str::<StorageEvent>()` 反序列化，按 `turn_id` 分组构建 `TurnTrace`，并在 session 级聚合 metadata / lineage。处理不完整 turn（无 `TurnDone`）、跨 agent 谱系事件以及 `AgentCollaborationFact` 到 `SubRunTrace`/turn 协作摘要的关联。验证：使用现有 session JSONL 文件做集成测试，确认提取的 turn 数量与手动检查一致。
- [ ] 1.4 实现 `ToolCallRecord` 生命周期构建：在 extractor 中将 `ToolCall` + `ToolCallDelta`(可选) + `ToolResult` 事件合并为完整的 `ToolCallRecord`。处理 `ToolResultReferenceApplied` 事件。验证：单元测试覆盖正常完成、有流式输出、被持久化引用替换三种场景。

## 2. 评测任务规范

- [ ] 2.1 定义 `EvalTask` 类型：在 `crates/eval/src/task/mod.rs` 中定义 `EvalTask`、`WorkspaceSpec`、`ExpectedOutcome`、`ToolPattern`、`FileChangeExpectation`、`ScoringWeights` 等结构体，支持 serde YAML 反序列化。验证：单元测试确保 YAML → `EvalTask` 反序列化正确。
- [ ] 2.2 实现 `TaskLoader`：在 `crates/eval/src/task/loader.rs` 中实现从目录加载任务文件和从 `task-set.yaml` 加载任务集索引。校验必要字段（`task_id`、`prompt`、`expected_outcome`），缺失时报错。验证：创建几个测试用 YAML 文件，确认加载成功和校验报错。
- [ ] 2.3 实现评分器：在 `crates/eval/src/task/scorer.rs` 中实现多维度匹配与归一化评分。维度包括：工具调用序列匹配、最大调用次数检查、文件变更验证（通过读取工作区文件内容匹配）、最大 turn 数检查。输出 `EvalScore`（0.0-1.0）和 `EvalStatus`（pass/partial/fail）。验证：针对各维度编写单元测试。

## 3. 失败模式诊断器

- [ ] 3.1 定义诊断器 trait 与注册机制：在 `crates/eval/src/diagnosis/mod.rs` 中定义 `FailurePatternDetector` trait、`FailureInstance`、`FailureSeverity`、`DiagnosisReport` 等类型；`FailureInstance` 包含 `confidence`、`storage_seq` 范围与结构化上下文。实现 `DiagnosisEngine`：注册多个检测器，对 `TurnTrace` 依次调用，汇总报告。验证：单元测试使用 mock 检测器确认注册和调用流程。
- [ ] 3.2 实现 `ToolLoopDetector`：在 `crates/eval/src/diagnosis/tool_loop.rs` 中检测同名工具连续调用 ≥ 3 次且参数相似的模式。使用简单的字符串相似度（Jaccard 或编辑距离）比较参数。验证：构造包含循环的 `TurnTrace` 和不含循环的 trace，分别测试正负例。
- [ ] 3.3 实现 `CascadeFailureDetector`：在 `crates/eval/src/diagnosis/cascade_failure.rs` 中检测连续 ≥ 2 次工具调用失败。区分"失败后重试成功"的正常行为。验证：单元测试覆盖连续失败、单次失败、失败后重试成功三种场景。
- [ ] 3.4 实现 `CompactInfoLossDetector`：在 `crates/eval/src/diagnosis/compact_loss.rs` 中检测 compact 后出现工具调用失败的模式。匹配失败原因中暗示信息丢失的关键词。验证：单元测试覆盖 compact 后失败、compact 后正常两种场景。
- [ ] 3.5 实现 `SubRunBudgetDetector`：在 `crates/eval/src/diagnosis/subrun_budget.rs` 中检测子 Agent 执行超过步数限制。从 `SubRunTrace` 中比较 `step_count` 与 `resolved_limits`。验证：单元测试。
- [ ] 3.6 实现 `EmptyTurnDetector`：在 `crates/eval/src/diagnosis/empty_turn.rs` 中检测无工具调用且助手输出为空的 turn。可配置最小输出长度阈值。验证：单元测试。

## 4. 评测运行器

- [ ] 4.1 实现 server 控制面客户端与 session log 定位：在 `crates/eval/src/runner/client.rs` 中封装创建 session、提交 turn 的 HTTP 调用；在 runner 模块中实现基于 `session_id` + `session_storage_root` 的 JSONL 定位与 `TurnDone` 轮询等待。依赖 `reqwest`。支持超时配置，并在无法访问共享 session 存储时明确报错。验证：集成测试（需启动 server）。
- [ ] 4.2 实现工作区管理：在 `crates/eval/src/runner/workspace.rs` 中实现 fixture 目录 copy 到隔离路径、评测后清理。支持 `--keep-workspace` 跳过清理。验证：集成测试确认目录创建和清理。
- [ ] 4.3 实现评测结果收集与报告生成：在 `crates/eval/src/runner/report.rs` 中将每个任务的结果（`EvalScore` + `DiagnosisReport` + 指标）汇总为 `EvalReport`。实现 JSON 序列化与持久化。验证：单元测试确认报告结构完整。
- [ ] 4.4 实现基线对比：在 `crates/eval/src/runner/report.rs` 中扩展报告生成逻辑，支持读取历史基线 JSON 文件并计算 diff。输出各任务分数变化、指标变化，分数下降超阈值时输出警告。验证：单元测试使用两个 mock 报告文件。
- [ ] 4.5 实现并行执行：在 `crates/eval/src/runner/mod.rs` 中使用 `tokio` 并发执行多个评测任务（`--concurrency` 参数控制）。每个任务使用独立 session，失败不影响其他任务。验证：集成测试。
- [ ] 4.6 创建 binary 入口：在 `crates/eval/src/bin/eval_runner.rs` 中实现 CLI 入口，解析参数（`--server-url`、`--session-storage-root`、`--task-set`、`--baseline`、`--concurrency`、`--keep-workspace`、`--output`）。编排完整执行流程，并在控制面可达但数据面不可达时快速失败。验证：`cargo run -p astrcode-eval -- --help` 正常输出。

## 5. 评测任务集与端到端验证

- [ ] 5.1 创建核心评测任务集目录 `eval-tasks/core/`，编写 3-5 个初始评测任务 YAML（覆盖文件读取、文件编辑、工具链效率等基本场景）。创建 `task-set.yaml` 索引。验证：`TaskLoader` 能成功加载全部任务。
- [ ] 5.2 端到端验证：启动 server，运行评测运行器执行核心任务集，确认完整流程（任务加载 → 工作区准备 → session 创建 → turn 提交 → trace 提取 → 诊断 → 评分 → 报告输出）畅通。验证：生成有效的 JSON 评测报告。
- [ ] 5.3 基线验证：运行两次相同评测集，第二次使用第一次结果作为基线，确认 diff 输出正确。验证：diff 报告显示无变化。
