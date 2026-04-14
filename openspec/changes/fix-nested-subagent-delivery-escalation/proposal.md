## Why

当前多级子代理链路在 `leaf -> middle -> root` 场景下暴露出两个相反的问题：

- 首轮 spawn 与显式 resume 会注册 child terminal watcher，但 wake 路径的语义边界不清晰
- 一旦把 wake turn 也当成新的 child work turn 自动向上冒泡，就会把“协调 turn”误提升为“新任务结果”，导致子代理链路自激膨胀

这会导致：

- 孙级子代理可以把结果交付给父级子代理
- 父级子代理能被 wake 并处理这条结果
- 但父级子代理这一轮结束后，不会再把自己的结果交付给主代理

问题根因不是提示词，而是 child turn 与 wake turn 的生命周期边界没有被清晰建模：

- 真正的 child work turn 需要统一 terminal finalizer
- wake turn 只是消费 mailbox 的协调 turn，不应再自动制造新的上行 delivery

## What Changes

- 在 `application` 中引入统一的 child turn terminal finalizer，覆盖 spawn 与 idle-resume 这两类真正的 child work turn。
- 明确 child turn 的异常收口语义：turn 失败/取消仍要向直接父级投递 terminal delivery；finalizer 自身失败时不得伪造成功消费。
- 切断 `ChildAgentRef` 与 parent routing truth 的耦合，内部显式使用 `parent_session_id / parent_turn_id` 路由 notification。
- 让 wake turn 保持为纯协调/消费 turn：只负责当前 batch 的 `acked / consume / requeue`，不再自动向更上一级制造 terminal delivery。
- 补齐多级回传、wake bubbling、路由隔离和“本轮独立收口”回归测试。

## Capabilities

### Modified Capabilities

- `subagent-execution`: child turn 的 terminal 结果必须通过统一 finalizer 收口，不得因入口不同而漏掉上行 delivery。
- `agent-delivery-contracts`: parent wake turn 只负责消费当前 direct-parent mailbox batch，不得把协调 turn 本身再包装成新的 child terminal delivery。

## Impact

- 影响代码：
  - `crates/application/src/agent/terminal.rs`
  - `crates/application/src/agent/wake.rs`
  - `crates/application/src/agent/mod.rs`
  - `crates/application/src/agent/routing.rs`
- 不修改外部 HTTP / protocol DTO 结构。
- 会调整内部 terminal notification id 生成规则，使其按 `sub_run_id + turn_id + status` 区分同一 agent 的多轮 turn。

## Non-Goals

- 不引入“等待整棵后代子树全部 settled 再统一回复”的树级聚合语义。
- 不在本次变更中重命名或 breaking 调整 `ChildAgentRef.session_id` 的外部字段。
- 不通过提示词约束来替代 runtime 级 delivery 编排。
