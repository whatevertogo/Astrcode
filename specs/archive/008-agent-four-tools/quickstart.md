# Quickstart: Astrcode Agent 协作四工具重构

## 目标

按不保留公开兼容层的前提，把当前协作系统切换到 `spawn/send/observe/close` 四工具模型，并完成 root agent 控制树接入、durable mailbox、Idle 生命周期和调用方迁移。

## 推荐实施顺序

1. **先改 core 契约**
   - 拆出 `AgentLifecycleStatus` / `AgentTurnOutcome`
   - 增加四工具 DTO
   - 定义 mailbox durable 事件与 `delivery_id`/`batch_id`
   - 保留内部 `resume` 预留，不暴露公开 surface

2. **再改 control/runtime 地基**
   - 注册 root agent 到 `agent_control`
   - 升级 `SubRunHandle` 语义
   - 新 child 写路径固定为 `IndependentSession`
   - 搭建 live inbox + durable mailbox replay

3. **实现 mailbox 调度**
   - `send` 路由与权限校验
   - `snapshot drain` + `BatchStarted`
   - durable turn completion 后 `BatchAcked`
   - `close` 写 `Discarded` 并清理 subtree wake item

4. **收敛工具、prompt 与调用面**
   - 替换 `runtime-agent-tool` 注册
   - 重写 `workflow_examples`
   - 更新 `runtime-agent-loop` mailbox 注入
   - 更新 `crates/server`、`frontend/src/hooks/useAgent.ts`、`frontend/src/lib/api/sessions.ts`

5. **最后删除旧协作面**
   - 删除旧 DTO / schema / tool 实现 / prompt 文案 / regression tests
   - 全局搜索确认旧工具名彻底消失

## 关键验证场景

### 场景 1：子 Agent 可复用

1. 父 agent 调用 `spawn(...)`
2. 子 agent 完成第一轮工作
3. `observe(childId)` 返回 `lifecycleStatus = Idle`
4. 父 agent 再次 `send(childId, ...)`
5. 子 agent 成功开始第二轮

### 场景 2：运行中消息延迟到下一轮

1. 让 child 进入 `Running`
2. 父 agent 连续发送两条消息
3. 当前轮只能消费 turn-start batch
4. 第二条消息保留到下一轮
5. `pendingMessageCount` 与 replay 结果一致

### 场景 3：重启后的消息恢复

1. 写入 `AgentMailboxQueued`
2. 写入 `AgentMailboxBatchStarted`
3. 在 `BatchAcked` 之前模拟重启
4. 恢复后相同 `delivery_id` 重新出现于 pending
5. 该行为被视为合法的 `at-least-once` 重放

### 场景 4：父级唤醒

1. child 调用 `send(parentId, "...")`
2. parent mailbox 成功入队
3. parent 空闲时被唤醒到下一轮
4. 当前轮 prompt 注入中包含对应 `delivery_id`

### 场景 5：权限与关闭

1. 非直接父调用 `observe(childId)` 被拒绝
2. 兄弟 agent `send` 被拒绝
3. `close(childId)` 后 subtree 全部 `Terminated`
4. 关闭后的 agent 再收 `send` 必须报错

## 必跑命令

### 仓库级验证

```powershell
cd D:\GitObjectsOwn\Astrcode
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cd frontend
npm run typecheck
```

### 旧工具名清理

```powershell
cd D:\GitObjectsOwn\Astrcode
rg -n "waitAgent|sendAgent|closeAgent|deliverToParent|resumeAgent" crates frontend -g '*.rs' -g '*.ts' -g '*.tsx'
```

### 建议的专项测试

```powershell
cd D:\GitObjectsOwn\Astrcode
cargo test -p runtime-agent-control
cargo test -p runtime-agent-tool
cargo test -p runtime-agent-loop
cargo test -p runtime collaboration
```

## 实施注意事项

- `BatchStarted` 必须是 mailbox-wake turn 的第一条 durable 事件，不能在它之前先写别的 turn 内事件。
- live inbox 只能在 `AgentMailboxQueued` append 成功后更新，顺序不能反过来。
- `observe` 中的 `pendingMessageCount` 以 durable replay 为准，不要依赖 live cache 作为真相。
- prompt 注入不是 durable transcript，不要假设 context window 一定还能看到上一次注入过的 `delivery_id`。
- `close` 当前只做 subtree terminate，不在本轮偷偷扩展成 detach / preserve descendants。
- 触达 `runtime-agent-control` 和 `runtime/src/service/execution` 时，必须额外审计锁使用、异步句柄生命周期和错误恢复路径，避免持锁跨 await 或 fire-and-forget 漏管。
