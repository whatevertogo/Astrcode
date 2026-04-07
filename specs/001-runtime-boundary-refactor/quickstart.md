# Quickstart: Runtime Boundary Refactor Validation

## 1. 全量验证命令

在实现完成后，先跑仓库基线验证：

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
Set-Location frontend
npm run typecheck
npm run test
npm run lint
npm run build
```

## 2. 建议的定向验证

用于快速确认本次重构最关键的协议与边界行为：

```powershell
cargo test -p astrcode-protocol subrun_event_serialization
cargo test -p astrcode-runtime-execution subrun
cargo test -p astrcode-runtime-agent-loader
cargo test -p astrcode-server session_history_endpoint_filters_subrun_scope_and_cursor
cargo test -p astrcode-server session_events_contract_rejects_scope_without_subrun_id
Set-Location frontend
npm run test -- subRunView
npm run test -- agentEvent
```

## 3. 验证矩阵与样本覆盖

| 验证目标 | 样本来源 | 关键测试/命令 |
|---------|---------|--------------|
| 协议事件序列化兼容性 | `crates/protocol/tests/fixtures/v4/initialize.json`、`invoke.json`、`event_delta.json`、`cancel.json`、`result_initialize.json`、`result_error.json` | `cargo test -p astrcode-protocol conformance` |
| history scope 过滤语义 | `crates/server/src/tests/runtime_routes_tests.rs::seed_shared_subrun_session()` | `cargo test -p astrcode-server session_history_endpoint_filters_subrun_scope_and_cursor` |
| scope 参数契约校验 | `crates/server/src/tests/session_contract_tests.rs` 中的 scope 参数契约测试 | `cargo test -p astrcode-server session_history_contract_rejects_scope_without_subrun_id` + `cargo test -p astrcode-server session_events_contract_rejects_scope_without_subrun_id` |
| legacy 历史降级行为 | 当前由 server 路由测试中的手工 durable 样本覆盖；协议 fixture 侧通过 `crates/protocol/tests/fixtures/README.md` 记录覆盖边界 | 场景 C 手工验收 + server/runtime 回归测试 |

## 4. 手工验收场景

### 场景 A: durable subrun lineage 在 live 清理后仍然成立

1. 从一个 session 提交 prompt，触发 `spawnAgent`，并确保产生 child subrun。
2. 记录运行中 `GET /api/v1/sessions/{id}/subruns/{sub_run_id}` 的返回值。
3. 等待 subrun 结束，清空或重启 live runtime。
4. 再次查询相同 subrun status。

**期望**

- `descriptor.subRunId`、`descriptor.parentTurnId`、`descriptor.parentAgentId`、`descriptor.depth` 不变。
- `toolCallId` 不因 live 清理而丢失。
- `source` 从 `live` 切换到 `durable`，但 durable facts 不变。

### 场景 B: `/history` 与 `/events` 对同一子树给出一致 scope

1. 准备包含根执行、直接子执行、孙级子执行的样本 session。
2. 分别调用：
   - `GET /api/sessions/{id}/history?subRunId={target}&scope=self`
   - `GET /api/sessions/{id}/history?subRunId={target}&scope=directChildren`
   - `GET /api/sessions/{id}/history?subRunId={target}&scope=subtree`
3. 对同一 target 再订阅 `GET /api/sessions/{id}/events?...`，比较 initial replay。

**期望**

- `/history` 与 filtered `/events` 的首段结果一致。
- `directChildren` 不包含 target 自身与孙级后代。
- `subtree` 包含 target 自身与所有递归后代。

### 场景 C: legacy 历史不再伪造 ancestry

1. 使用缺少新 `descriptor` 的旧 session log 样本。
2. 调用 `scope=self`、`scope=directChildren`、`scope=subtree`。
3. 查询 subrun status。

**期望**

- `scope=self` 仍可返回结果。
- `scope=directChildren` 和 `scope=subtree` 返回显式错误。
- status 返回 `source=legacyDurable`，并把缺失字段保留为 `null`。

### 场景 D: working-dir 解析跟随 execution context

1. 准备两个不同工作目录 `repo-a` 与 `repo-b`，它们各自定义同名 agent。
2. 对 `POST /api/v1/agents/{id}/execute` 分别传入两个 `workingDir`。
3. 观察执行到的 agent profile 和热重载路径。

**期望**

- 两次 root execution 解析到各自项目内的 agent 定义。
- 修改 `repo-a` 的 agent 文件，只影响 `repo-a` resolver，不影响 `repo-b`。
- 缺少 `workingDir` 时返回 `400`，而不是静默使用进程 cwd。

### 场景 E: frontend subrun tree 与 server lineage 一致

1. 刷新页面，只通过 `/history` 重建完整消息流。
2. 进入同一 session 的 subrun tree。
3. 再让 SSE 接入补齐新的 child / grandchild。

**期望**

- `frontend/src/lib/subRunView.ts` 生成的 parent/child 关系与后端 descriptor 一致。
- 刷新前后的 tree 结构一致。
- UI 不再依赖 `parentTurnId -> turn owner` 的启发式关系。

