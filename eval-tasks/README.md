# eval-tasks

`eval-tasks/` 是 `astrcode-eval` 的评测任务目录，负责定义可重复执行的核心评测集。

## 目录结构

```text
eval-tasks/
├── task-set.yaml          # 任务集索引
├── core/                  # 评测任务 YAML
└── fixtures/              # 每个任务对应的初始工作区快照
```

当前 `task-set.yaml` 会加载：

- `core/file-read-accuracy.yaml`
- `core/file-edit-precision.yaml`
- `core/tool-chain-efficiency.yaml`

## 任务文件约定

每个任务文件使用 YAML，核心字段如下：

- `task_id`: 全局唯一的 kebab-case 标识
- `description`: 任务说明
- `prompt`: 提交给 agent 的评测提示词
- `workspace.setup`: fixture 目录
- `expected_outcome`: 期望行为约束

示例：

```yaml
task_id: tool-chain-efficiency
description: 读取文档后更新计划文件，检验工具链效率。
prompt: |
  请先读取 docs/plan.md，再把 status.txt 改成 done。
workspace:
  setup: ../fixtures/tool-chain-efficiency
expected_outcome:
  tool_pattern:
    - Read
    - Edit
  max_tool_calls: 3
  file_changes:
    - path: status.txt
      exact: "done\n"
  max_turns: 1
```

## fixture 约定

`workspace.setup` 相对任务文件所在目录解析。

例如 `core/tool-chain-efficiency.yaml` 中：

```yaml
workspace:
  setup: ../fixtures/tool-chain-efficiency
```

最终会定位到：

`eval-tasks/fixtures/tool-chain-efficiency`

runner 会在执行前把 fixture 复制到隔离工作区，再把 session 的 `working_dir` 指向该隔离目录，因此 fixture 本身不应被直接修改。

## 隔离工作区

默认情况下，runner 会把 fixture 复制到系统临时目录下的：

`<temp>/astrcode-eval-workspaces/<task_id>-<timestamp>/`

评测结束后默认清理。若需要保留现场排查问题，使用：

```bash
cargo run -p astrcode-eval -- \
  --server-url http://127.0.0.1:5529 \
  --session-storage-root <projects-root> \
  --task-set eval-tasks/task-set.yaml \
  --keep-workspace
```

## 运行方式

```bash
cargo run -p astrcode-eval -- \
  --server-url http://127.0.0.1:5529 \
  --session-storage-root <projects-root> \
  --task-set eval-tasks/task-set.yaml \
  --output eval-report.json
```

如需做基线对比：

```bash
cargo run -p astrcode-eval -- \
  --server-url http://127.0.0.1:5529 \
  --session-storage-root <projects-root> \
  --task-set eval-tasks/task-set.yaml \
  --baseline eval-report.json \
  --output eval-report-next.json
```

## 维护规则

- 新任务必须同步补对应 fixture。
- 优先覆盖框架行为，不在这里做语义 judge。
- `expected_outcome` 应尽量具体，避免模糊描述。
- 若任务依赖文件变更验证，优先使用 `contains` 或 `exact` 明确断言。
- 修改 `task-set.yaml` 后，应确保 `TaskLoader` 能完整加载整个任务集。
