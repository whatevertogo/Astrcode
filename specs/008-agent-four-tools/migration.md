# Migration: 四工具协作面切换顺序与调用方清单

> **状态**: 迁移已完成。Phase 1-6 全部落地，旧六工具公开面已彻底删除。

## 迁移目标

在不保留公开兼容层的前提下，把当前协作系统从旧六工具模型切换到四工具模型，并保证 durable mailbox、Idle 生命周期和调用方命名同步完成。

## 调用方与影响面清单

### Core / DTO / 协议

| Path | Status | Change |
|------|--------|--------|
| `crates/core/src/agent/mod.rs` | ✓ 完成 | 拆出 lifecycle/outcome 子模块，新增四工具 DTO 与 mailbox durable 事件 |
| `crates/core/src/action.rs` | ✓ 完成 | 已迁移到四工具心智描述 |

### Runtime / Control / Tool

| Path | Status | Change |
|------|--------|--------|
| `crates/runtime-agent-control/src/lib.rs` | ✓ 完成 | root 已注册，lifecycle/outcome 独立管理，inbox 升级 |
| `crates/runtime-agent-tool/src/*` | ✓ 完成 | 四工具实现（spawn/send/observe/close） |
| `crates/runtime/src/builtin_capabilities.rs` | ✓ 完成 | 只注册四工具 |
| `crates/runtime/src/service/execution/collaboration.rs` | ✓ 完成 | 按 mailbox durable 语义重写 |
| `crates/runtime/src/service/execution/root.rs` | ✓ 完成 | root agent 已接入控制树 |
| `crates/runtime-agent-loop/src/subagent.rs` | ✓ 完成 | Agent Mailbox Batch 注入 |
| `crates/runtime-prompt/src/contributors/workflow_examples.rs` | ✓ 完成 | 重写为四工具协作心智 |
| `crates/runtime-session/src/*` | ✓ 完成 | mailbox 事件追加与 replay |

### Server / Frontend / Tests

| Path | Status | Change |
|------|--------|--------|
| `crates/server/src/tests/session_contract_tests.rs` | ✓ 完成 | 四工具合同验证 |
| `frontend/src/hooks/useAgent.ts` | ✓ 完成 | 使用新 close 调用面 |
| `frontend/src/lib/api/sessions.ts` | ✓ 完成 | 四工具命名与响应类型 |
| `crates/runtime-agent-tool/src/tests.rs` | ✓ 完成 | 四工具行为测试 |
| `crates/runtime-agent-loop/src/agent_loop/tests/regression.rs` | ✓ 完成 | 四工具事件序列 |

## 建议迁移顺序

### Phase 1: 契约先行

- 改 `core`
- 写明新 lifecycle/outcome
- 增加 mailbox durable 事件
- 保留内部 resume 预留

**Why**:
- 其他边界都依赖统一 DTO 和事件定义

### Phase 2: 控制树与新写路径

- root 注册进 `agent_control`
- `SubRunHandle` 升级为持久 agent 句柄语义
- spawn 新 child 统一走 `IndependentSession`

**Why**:
- 没有真实 root/child 树，就无法正确实现 `send/observe/close`

### Phase 3: durable mailbox 与 replay

- append `Queued/Started/Acked/Discarded`
- 建 mailbox projector / replay
- 接上 live inbox/cache 更新顺序

**Why**:
- 四工具模型的可靠性基础在于 pending message 可恢复

### Phase 4: runtime 执行逻辑替换

- `send` 权限与调度
- `observe` 快照聚合
- `close` subtree terminate
- `snapshot drain` 与 ack 顺序

**Why**:
- 只有 runtime 逻辑稳定后，公开工具和 prompt 才能安全切换

### Phase 5: 工具面与 prompt 切换

- 替换 `runtime-agent-tool`
- 更新 `builtin_capabilities`
- 重写 few-shot / tool description / mailbox prompt injection

**Why**:
- 避免模型在中途混用新旧工具语义

### Phase 6: 调用方与测试迁移

- server
- frontend
- runtime/tool/loop regression tests
- 旧工具名全局清理

**Why**:
- 公开面一旦切换，调用方必须同步完成，不保留 shim

## 明确不做的兼容策略

- 不提供旧工具名到新工具名的转发 ✓
- 不保留 `waitAgent` 的”临时 deprecated”阶段 ✓
- 不让新 child 继续写入 `SharedSession` ✓

## 风险与缓解

| Risk | Impact | Mitigation |
|------|--------|------------|
| `BatchStarted` 后崩溃导致重复投递 | 同一 `delivery_id` 重新出现 | 明确采用 `at-least-once`，并在 prompt 中暴露 `delivery_id` |
| root 注册遗漏 | 根层 send/observe 权限失真 | 先做 root control tree 改造，再迁工具面 |
| 调用方漏改 | 前端/server 继续暴露旧名字 | 全局 `rg` 搜索 + tests 收尾 |
| mailbox 状态混入旧 projector | 职责边界变脏 | 单独 mailbox projector/replay |

## 收尾验证

```powershell
cd D:\GitObjectsOwn\Astrcode
rg -n "waitAgent|sendAgent|closeAgent|deliverToParent|resumeAgent" crates frontend -g '*.rs' -g '*.ts' -g '*.tsx'

cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
