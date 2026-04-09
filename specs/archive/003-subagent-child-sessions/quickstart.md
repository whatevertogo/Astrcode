# Quickstart: 子 Agent Child Session 重构验证

## 1. 全量验证命令

实现完成后先跑仓库基线验证：

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

用于快速确认本次 feature 最关键的 child session / collaboration 行为：

```powershell
cargo test -p astrcode-runtime-agent-tool
cargo test -p astrcode-runtime-registry
cargo test -p astrcode-runtime-agent-loop
cargo test -p astrcode-runtime
cargo test -p astrcode-server
Set-Location frontend
npm run test -- subRunView
npm run test -- agentEvent
```

若实现中新增了 child-session projection、agent inbox 或 collaboration tool 专项测试，优先再补以下定向命令：

```powershell
cargo test -p astrcode-runtime child_session
cargo test -p astrcode-runtime collaboration
cargo test -p astrcode-server child_session
Set-Location frontend
npm run test -- childSession
```

## 3. 验证矩阵

| 验证目标 | 关键命令 / 场景 |
|---------|----------------|
| `spawnAgent` 与后续协作工具的 schema / 结果投影稳定 | `cargo test -p astrcode-runtime-agent-tool` |
| `CapabilityRouter` 成为唯一生产执行入口，tool/capability 上下文透传正确 | `cargo test -p astrcode-runtime-registry` |
| parent turn 结束后 child session 继续存活，完成后能重新激活 parent | `cargo test -p astrcode-runtime` + 手工场景 B |
| server 历史 / events / status / child routes 投影一致 | `cargo test -p astrcode-server` |
| 前端父摘要 / 子会话完整视图切换正确，不展示 raw JSON | `cd frontend && npm run test -- childSession` + 手工场景 D |

## 4. 手工验收场景

### 场景 A: spawn 立即返回，child session 成为独立会话

1. 在主会话触发 `spawnAgent`。
2. 记录返回的 child ref、child session id、parent summary。
3. 观察 child session 是否能在独立入口打开。

**期望**

- `spawnAgent` 快速返回，不等待 child 完成。
- parent 会话只收到一个可消费的启动摘要，而不是 child 原始中间事件流。
- child session 可以通过稳定 `sessionId` 打开。

### 场景 B: 父 turn 结束后 child 完成，并重新唤醒 parent

1. 触发一个耗时 child task。
2. 在 child 完成前结束父 turn。
3. 等待 child 完成并观察 parent 是否继续工作。

**期望**

- child 不会因父 turn 结束而被取消。
- child 最终交付以 tool 结果/任务通知投影进入 parent。
- parent 在需要时被重新激活，而不是只留下静态通知。

### 场景 C: 主子协作闭环

1. `spawnAgent` 创建 child。
2. 对同一个 child 执行补充要求（send）。
3. 对该 child 执行等待（wait）。
4. 对同一个 child 执行关闭或恢复（close/resume）。

**期望**

- 所有协作都通过统一 tool surface。
- runtime 内部只发生一次有效投递与一次有效消费。
- close 只影响目标 child 子树，不影响平级 child。

### 场景 D: 父摘要视图与子完整视图分离

1. 在父会话中观察多个 child 的摘要列表。
2. 展开其中一个 child，进入完整 child session。
3. 刷新页面后再次打开同一个 child。

**期望**

- 父会话只显示摘要、工具活动概览和最终回复摘录。
- 子会话显示完整 thinking、tool activity、最终回复。
- 全流程不展示 raw JSON。
- 刷新后仍能打开同一个 child session，而不是重新推断 thread tree。

### 场景 E: 恢复、重试和重复通知不产生双消费

1. 触发 child 通知后，模拟 SSE 重连、session reload 或 runtime 重启。
2. 再次观察 parent 是否收到重复交付。

**期望**

- 相同 `dedupe_key` 的协作消息只被有效消费一次。
- parent 不会因为 replay 或 reconnect 重复执行同一协作结果。
- status source 会明确区分 `live`、`durable` 和 `legacyDurable`。
