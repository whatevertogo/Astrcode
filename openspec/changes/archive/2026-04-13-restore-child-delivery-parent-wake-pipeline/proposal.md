## Why

当前仓库虽然保留了 parent delivery queue、wake service 与相关控制合同，但 child completion 到 parent wake 的真实接线没有形成闭环，导致子代理完成后结果回流父级的行为只停留在设计与局部实现上。这个问题会直接削弱四工具模型的可组合性，也会让现有 specs 与运行时真实行为继续分叉。

## What Changes

- 恢复 child terminal delivery 到 parent wake 的完整管线，使子代理完成、失败或关闭后都能通过正式 notification 与 delivery batch 机制驱动父级后续执行。
- 明确 child completion 的事件产生边界，确保 `application` 与 `session-runtime` 在“谁负责生成 notification、谁负责调度 wake”上职责单一、可追踪。
- 将现有 `reactivate_parent_agent_if_idle`、delivery queue checkout / consume / requeue 逻辑纳入真实运行路径，而不是仅保留为未接线能力。
- **BREAKING** 收紧父级回流语义：child 终态不再允许只停留在子会话内部结果或 UI 投影层，必须经过稳定 delivery 合同进入父级事实链路。
- 为失败与繁忙路径定义稳定行为，包括父级繁忙时的排队重试、wake 提交失败时的 requeue 与可观测日志/指标更新。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `agent-delivery-contracts`: child delivery、wake、close、observe 的稳定控制合同需要从“可调用能力”升级为真实运行闭环。
- `subagent-execution`: 子代理完成后必须通过正式 delivery / wake 管线回流父级，而不是只返回局部执行结果。
- `agent-execution`: root/subagent 执行的协作语义需要补齐 parent delivery 的终态推进与失败回滚。
- `agent-lifecycle`: child 终态、parent wake 与 delivery 缓冲之间的生命周期推进需要更明确的行为约束。

## Impact

- 影响代码：
  - `crates/application/src/agent/wake.rs`
  - `crates/application/src/execution/subagent.rs`
  - `crates/application/src/agent/mod.rs`
  - `crates/kernel/src/agent_tree/mod.rs`
  - `crates/session-runtime/src/turn/*`
- 影响系统：
  - child terminal notification 生成链路
  - parent delivery queue 消费与重试
  - 相关 observability 指标与日志
- 影响用户可见行为：
  - 父 agent 将重新看到子 agent 终态回流
  - busy / failed wake 情况下的表现更稳定、更可诊断

## Non-Goals

- 本次不重构整个 agent mailbox 数据模型，也不引入新的 typed actor 框架。
- 本次不处理 root/subagent 执行入口与 profile 解析收口；那是独立 change 的范围。
- 本次不处理 profile 文件监听与 cache invalidation。

## Migration And Rollback

- 迁移方式为“先补回调用链，再统一终态语义”：优先把现有 wake service 接到 child completion 出口，再补充失败回滚与指标一致性。
- 如果新 wake 管线在集成阶段造成父会话重复唤醒或 delivery 丢失风险，可临时回滚到“只保留 queue、不自动 wake”的保守模式，但必须同时撤回相应 spec 变更并明确降级行为。
