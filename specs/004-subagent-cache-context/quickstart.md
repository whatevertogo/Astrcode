# Quickstart: 子智能体会话与缓存边界优化验证

## 1. 全量验证命令

实现完成后先跑仓库基线验证：

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
Set-Location frontend
npm run typecheck
npm run lint
npm run format:check
npm run test
Set-Location ..
```

## 2. 建议的定向验证

先跑与本 feature 最相关的 crate：

```powershell
cargo test -p astrcode-runtime-session
cargo test -p astrcode-runtime-execution
cargo test -p astrcode-runtime-agent-control
cargo test -p astrcode-runtime-agent-loop
cargo test -p astrcode-runtime-prompt
cargo test -p astrcode-runtime
cargo test -p astrcode-server
```

如果前端调整了父摘要或子会话入口，再跑：

```powershell
Set-Location frontend
npm run typecheck
npm run test
Set-Location ..
```

如果想先快速抽查 004 的关键回归，再补全全量矩阵，可以优先跑：

```powershell
cargo test -p astrcode-runtime --lib get_subrun_status_rejects_legacy_shared_history_snapshots
cargo test -p astrcode-runtime parent_history_contract_hides_independent_subrun_lifecycle_and_keeps_notifications
cargo test -p astrcode-runtime-session --lib session_state_rehydrates_child_nodes_from_stored_notifications
Set-Location frontend
npm test -- --run src/lib/subRunView.test.ts src/lib/sessionHistory.test.ts src/lib/sessionView.test.ts src/lib/agentEvent.test.ts
Set-Location ..
```

## 3. 手工验收场景

### 场景 A: 连续创建多个子智能体，父历史保持干净

1. 在同一父会话下连续触发至少 10 次子任务创建。
2. 观察每个子任务是否拿到不同的 `child_session_id`。
3. 检查父会话历史。

**期望**

- 每个新子智能体都有独立子会话。
- 父历史里只出现 started / delivered / completed / failed / cancelled 等边界事实。
- 父历史里不出现来自子会话的 `AssistantFinal`、`ToolCall`、`ToolResult`、`PromptMetrics`、`TurnDone` 等内部事件。

### 场景 B: 中断后 resume，沿用原子会话继续

1. 让某个 child 运行到中途后暂停或模拟进程重启。
2. 对该 child 执行 resume。
3. 观察恢复后的 `child_session_id` 与新的 `execution_id`。

**期望**

- `child_session_id` 与恢复前一致。
- `execution_id` 为新的执行实例。
- child 能基于原有消息历史和阶段继续，而不是像新 spawn 一样从空状态开始。

### 场景 C: 继承背景进入 system blocks，而不是首条任务消息

1. 在父会话中制造明显的 compact summary 与 recent tail。
2. 启动相似 child。
3. 观察 child 的首条任务消息和 prompt 构建结果。

**期望**

- 首条任务消息只描述任务目标和直接上下文。
- 父背景通过独立 inherited blocks 进入 system prompt。
- recent tail 经过裁剪，不直接继承超长工具原文。

### 场景 D: 重复启动相似 child，观察缓存收益

1. 在支持缓存指标的 provider 上，保持 working dir、profile、工具集合和规则输入不变。
2. 连续启动多次相似 child。
3. 比较首次与后续的 `PromptMetrics.cache_creation_input_tokens`。

**期望**

- 后续 child 的 `cache_creation_input_tokens` 相较首次下降至少 70%。
- 只要相关输入变化，缓存就会失效而不是误命中。

### 场景 E: 子交付在父 turn 结束后可靠唤醒父，但不污染 durable 历史

1. 启动多个 child，并在父 turn 结束后让它们依次交付。
2. 观察父是否继续处理这些交付。
3. 检查父 durable 历史。

**期望**

- 父通过运行时信号被唤醒继续处理。
- 多个交付会独立排队、逐个消费。
- 父 durable 历史不出现 `ReactivationPrompt` 或携带交付详情的机制性 `UserMessage`。
- 如果 SSE 在 lagged recovery 期间失败，前端能收到结构化 `error` 事件，而不是静默断流。

### 场景 F: 旧共享写入历史被显式拒绝

1. 准备一个旧共享写入模式的 session 样本。
2. 尝试通过读取、回放或恢复入口访问它。
3. 观察返回值和日志。

**期望**

- 系统返回稳定错误码，例如 `unsupported_legacy_shared_history`。
- 返回信息明确提示 `upgrade required` 或 `cleanup required`。
- 不会再进入共享写入兼容投影或恢复路径。

## 4. 快速排查提示

- 如果新 child 仍写进父历史，优先检查独立子会话创建路径是否仍被 experimental guard 拦截。
- 如果 resume 看起来成功但行为像新 spawn，优先检查是否真正走了 child session durable replay。
- 如果缓存收益不稳定，优先检查 inherited blocks 是否仍混入消息流，以及 provider 是否真正产出缓存 telemetry。
- 如果父唤醒丢交付，先区分是运行时缓冲丢失，还是 durable 边界事实没有落盘。
- 如果旧 session 仍被当作可读 child 历史处理，优先检查 legacy 拒绝路径是否被彻底切断。
- 如果前端看不到父摘要，但子会话详情页正常，优先检查是否误回到了 mixed-thread helper，而不是 `childSessionNotification` 摘要投影。
