## Context

Astrcode 的 `StorageEvent` JSONL 事件日志已经完整记录了 Agent 运行时的每一个关键动作（工具调用、LLM 输出、token 指标、compaction、子 Agent 生命周期）。`runtime-observability-pipeline` 和 `agent-tool-evaluation` 两个已有 spec 已经建立了实时指标采集和 agent 协作评估的基础。

当前缺失的是：**离线评测层** — 将已完成 session 的事件流转化为可度量、可对比、可诊断的结构化评测数据，用于驱动框架迭代。

本次设计明确区分两条通路：

- **控制面**：通过现有 `astrcode-server` HTTP API 创建 session、提交 turn
- **数据面**：通过本地共享 session 存储中的 JSONL 读取 durable 事件并提取 trace

因此本次评测运行器的适用场景是本机开发与 CI 共置部署；不覆盖仅能访问远端 HTTP API、但无法访问对应 session 存储目录的纯远程部署。

核心数据流：

```
StorageEvent JSONL
       │
       ▼
  ┌─────────────┐     ┌────────────────┐
  │ Trace       │────▶│ Failure        │
  │ Extractor   │     │ Diagnosis      │
  └──────┬──────┘     └───────┬────────┘
         │                    │
         ▼                    ▼
  ┌─────────────┐     ┌────────────────┐
  │ TurnTrace   │     │ Diagnosis      │
  │ Model       │     │ Report         │
  └──────┬──────┘     └───────┬────────┘
         │                    │
         ▼                    ▼
  ┌──────────────────────────────────┐
  │          Eval Runner             │
  │  (task spec → execute → score)   │
  └──────────────┬───────────────────┘
                 │
                 ▼
  ┌──────────────────────────────────┐
  │       Eval Result + Baseline     │
  │       (JSON report + diff)       │
  └──────────────────────────────────┘
```

## Goals / Non-Goals

**Goals:**

- 从现有 JSONL 日志中零运行时改动地提取结构化评测数据
- 定义标准化的评测任务规范，支持可重复、可对比的评测执行
- 自动检测 Agent 失败模式并生成可复现诊断报告
- 提供并行评测运行器，支持 CI 回归

**Non-Goals:**

- 不构建 LLM-as-Judge 语义评测（后续迭代）
- 不构建前端评测仪表板（后续迭代）
- 不引入容器化隔离（使用文件系统 copy）
- 不修改现有 StorageEvent 格式或运行时行为
- 不做跨模型能力对比评测

## Decisions

### D1: 独立 crate `crates/eval`，不侵入现有运行时

**选择**：新建独立 crate `astrcode-eval`，仅依赖 `core`（复用 `StorageEvent` serde 反序列化）和 `protocol`（复用 HTTP DTO 调用 server API）。

**替代方案**：在 `application` 或 `session-runtime` 中增加评测逻辑。
**否决原因**：评测是离线分析工具，不是运行时职责。侵入运行时违反架构分层原则，且评测逻辑变动不应影响线上行为。

**依赖方向**：`eval → core + protocol`，不反向依赖。符合 PROJECT_ARCHITECTURE.md 中 `adapter-*/独立工具 → core` 的模式。

### D2: Trace 提取基于 JSONL 文件直读，而非 server API replay

**选择**：评测 trace 提取器直接读取 JSONL 文件，通过 serde 反序列化为 `StorageEvent`，再转换为 `SessionTrace`（内含 `Vec<TurnTrace>`）。

**替代方案**：通过 server `/sessions/:id/events` API 获取事件。
**否决原因**：
1. 需要启动 server 实例，增加评测环境复杂度
2. API 返回的是 `AgentEvent`（面向 SSE），缺少部分 durable 字段
3. JSONL 是 ground truth，直接读取更简单、更可靠

**数据流**：
```
文件路径 → 逐行读取 → serde_json::from_str::<StorageEvent>()
         → TurnTraceBuilder / SessionTraceBuilder 累积
         → 输出 SessionTrace { metadata, turns, lineage }
```

### D3: 评测任务规范使用 YAML 格式

**选择**：评测任务定义使用 YAML 文件，每个任务一个文件，按目录组织任务集。

**替代方案**：Rust 代码定义任务（类型安全）、JSON 格式（机器友好）、TOML（Rust 生态常用）。
**选择 YAML 原因**：
1. 人类可读性好，方便非 Rust 开发者编写和修改任务
2. 支持多行字符串（system prompt、expected output）
3. 前端生态（VS Code YAML 插件）成熟，编辑体验好
4. serde_yaml 已在 Rust 生态中广泛使用

**目录结构**：
```
eval-tasks/
├── core/                      # 核心评测任务集
│   ├── file-read-accuracy.yaml
│   ├── file-edit-precision.yaml
│   └── tool-chain-efficiency.yaml
├── regression/                # 回归测试任务集
│   └── compact-info-loss.yaml
└── task-set.yaml              # 任务集索引（引用上述文件）
```

### D4: 失败诊断采用规则引擎模式，不使用 LLM

**选择**：失败模式检测基于确定性规则，对 `TurnTrace` 做单 pass 扫描。

**替代方案**：调用 LLM 分析 turn trace 语义。
**否决原因**：
1. 规则引擎可复现、无成本、速度快
2. 绝大多数框架级失败模式（工具循环、级联失败、compaction 丢失）可以通过事件模式检测
3. 语义级诊断留作 P3 迭代，与规则引擎正交不冲突

**模式检测设计**：每个 `FailurePatternDetector` 是一个 trait 实现：
```rust
trait FailurePatternDetector: Send + Sync {
    /// 检测名称，用于报告输出
    fn name(&self) -> &str;
    /// 严重级别
    fn severity(&self) -> FailureSeverity;
    /// 在单个 turn trace 上检测，返回匹配到的实例（可能多个）
    fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance>;
}
```

内置检测器：
| 检测器 | 输入信号 | 检测逻辑 |
|--------|---------|---------|
| `ToolLoopDetector` | `ToolCallRecord` 序列 | 同名工具连续调用 ≥3 次，且参数相似度 > 阈值 |
| `CascadeFailureDetector` | `ToolResult` 序列 | 连续 ≥2 次工具调用失败 |
| `CompactInfoLossDetector` | `CompactApplied` + 后续 `Error` | compact 后紧接着工具调用失败 |
| `SubRunBudgetDetector` | `SubRunStarted/Finished` | step_count 超过 resolved_limits 阈值 |
| `EmptyTurnDetector` | `TurnTrace` 整体 | turn 结束但无工具调用且 assistant output 为空 |

### D5: 评测运行器通过 HTTP 控制面 + 本地 JSONL 数据面驱动

**选择**：评测运行器是一个独立 binary（`astrcode-eval-runner`），通过 HTTP API 与 `astrcode-server` 交互完成 session/turn 生命周期控制，通过共享 session 存储根目录读取 durable JSONL。

**约束**：`--server-url` 只负责控制面；运行器还必须能够解析并访问对应的本地 session 存储根目录（CLI 参数 `--session-storage-root`，默认使用标准项目级 session 存储规则）。

**执行流程**：
```
1. 连接一个现有 server 实例
2. 每个评测任务：
   a. 准备工作区：cp -r fixtures/<task> → /tmp/eval-{id}/
   b. 创建 session（POST /sessions，working_dir 指向隔离工作区）
   c. 提交 turn（POST /sessions/:id/turn，body = task.prompt）
   d. 基于 `session_id` + `session_storage_root` 定位本地 JSONL 文件
   e. 轮询 JSONL，等待 `TurnDone` durable 事件
   f. 读取 JSONL trace 并构建 `SessionTrace`
   g. 运行失败诊断
   h. 与 expected_outcome 对比评分
   i. 收集结果
3. 汇总所有任务结果，与基线对比
4. 输出评测报告
```

**并行策略**：同一 server 实例内通过不同 session 隔离（session 已绑定独立 working_dir），无需多实例。若无法访问共享 session 存储，则运行器应在启动阶段直接失败，而不是退化为不稳定的 SSE 轮询。

### D6: 评测结果使用 JSON 格式持久化

**选择**：评测结果存储为 JSON 文件，按时间戳 + commit SHA 命名。

```json
{
  "commit": "895809c0",
  "timestamp": "2026-04-20T10:00:00Z",
  "task_set": "core",
  "results": [
    {
      "task_id": "file-edit-precision",
      "status": "pass",
      "score": 0.85,
      "metrics": { "tool_calls": 3, "duration_ms": 4500, "tokens_used": 1200 },
      "failures": []
    }
  ],
  "summary": { "pass_rate": 0.9, "avg_score": 0.82 }
}
```

**基线对比**：指定一个历史结果文件作为 baseline，计算各指标的 diff。

## Risks / Trade-offs

### [Risk] 评测任务集维护成本
评测任务的质量直接决定评测体系的价值。任务集需要随框架迭代持续更新，否则会失效。
**缓解**：初始聚焦 10-15 个核心任务，覆盖最高频的 Agent 行为模式。每个框架改动同步更新受影响的任务。

### [Risk] 规则诊断器的误报/漏报
规则引擎可能对复杂场景产生误判。
**缓解**：诊断器输出 `severity` + `confidence` 字段，支持配置检测阈值。初始阶段以高精确度优先（宁可漏报不要误报），逐步扩展模式库。

### [Risk] 评测运行器对本地 session 存储布局有耦合
运行器除了依赖 server HTTP API，还需要与 session JSONL 的存储布局保持一致。如果存储路径规则变化，运行器需要同步更新。
**缓解**：通过显式 `--session-storage-root` 参数收敛路径来源，并尽量复用现有 session 路径规则，而不是在 runner 内散落隐式拼接逻辑。

### [Trade-off] 规则诊断 vs LLM 诊断
规则引擎无法覆盖语义级错误（如"代码逻辑正确但不符合用户意图"）。
**接受**：Phase 1 的目标是框架行为诊断（工具效率、token 消耗、失败模式），语义诊断留给后续 LLM-as-Judge 迭代。两者正交，不存在架构冲突。

### [Trade-off] 文件系统隔离 vs 容器隔离
文件系统 copy 无法隔离网络、进程等系统资源。
**接受**：当前评测不涉及需要强隔离的场景（如执行不可信代码）。如果后续需要，可以引入容器化而不影响评测框架的上层抽象。

## 文件变更清单

### 新增文件
```
crates/eval/
├── Cargo.toml
├── src/
│   ├── lib.rs                    # crate 入口
│   ├── trace/
│   │   ├── mod.rs                # TurnTrace 等核心类型
│   │   └── extractor.rs          # JSONL → TurnTrace 提取器
│   ├── task/
│   │   ├── mod.rs                # EvalTask 规范类型
│   │   └── loader.rs             # YAML 任务加载器
│   ├── diagnosis/
│   │   ├── mod.rs                # 诊断器 trait + 注册
│   │   ├── tool_loop.rs          # 工具循环检测
│   │   ├── cascade_failure.rs    # 级联失败检测
│   │   ├── compact_loss.rs       # compact 信息丢失检测
│   │   ├── subrun_budget.rs      # 子 Agent 预算超支检测
│   │   └── empty_turn.rs         # 空 turn 检测
│   ├── runner/
│   │   ├── mod.rs                # 评测运行器
│   │   ├── client.rs             # server HTTP API 客户端
│   │   ├── workspace.rs          # 工作区准备与清理
│   │   └── report.rs             # 结果汇总与基线对比
│   └── bin/
│       └── eval_runner.rs        # 独立 binary 入口
eval-tasks/
├── core/
│   └── ...                       # 核心评测任务 YAML
└── task-set.yaml                 # 任务集索引
```

### 修改文件
```
Cargo.toml                        # workspace members 加入 eval
```

### 不修改的文件
- 不修改 `core`、`session-runtime`、`application`、`server` 中任何文件
- 不修改 `StorageEvent` 或 `AgentEvent` 类型定义
- 不修改前端代码

## 可观测性

- 评测运行器输出结构化日志（tracing），记录每个任务的执行进度
- 评测结果 JSON 文件是唯一的评测 truth，不依赖内存状态
- 诊断报告中的 `storage_seq` 范围允许精确回溯到原始 JSONL 事件
