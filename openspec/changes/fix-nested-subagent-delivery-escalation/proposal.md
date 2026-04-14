## Why

当前多级子代理链路在 `leaf -> middle -> root` 场景下存在明显断链：首轮 spawn 与显式 resume 会注册 child terminal watcher，但 parent wake turn 在消费完下级 delivery 后，没有继续把当前 agent 这一轮的 terminal 结果正式向上级回传。

这会导致：

- 孙级子代理可以把结果交付给父级子代理
- 父级子代理能被 wake 并处理这条结果
- 但父级子代理这一轮结束后，不会再把自己的结果交付给主代理

问题根因不是提示词，而是 child turn 生命周期收口分散在多条入口上，导致 wake 路径漏掉了统一 terminal delivery。

## What Changes

- 在 `application` 中引入统一的 child turn terminal finalizer，覆盖 spawn、idle-resume、wake 三条 child turn 入口。
- 明确 child turn 的异常收口语义：turn 失败/取消仍要向直接父级投递 terminal delivery；finalizer 自身失败时不得伪造成功消费。
- 切断 `ChildAgentRef` 与 parent routing truth 的耦合，内部显式使用 `parent_session_id / parent_turn_id` 路由 notification。
- 让 wake turn 在完成 mailbox batch 收口前，先尝试把当前 agent 这一轮继续向上一级冒泡。
- 补齐多级回传、wake bubbling、路由隔离和“本轮独立收口”回归测试。

## Capabilities

### Modified Capabilities

- `subagent-execution`: child turn 的 terminal 结果必须通过统一 finalizer 收口，不得因入口不同而漏掉上行 delivery。
- `agent-delivery-contracts`: parent wake turn 成功或失败后，只要当前 agent 这一轮进入 terminal 状态，就必须按直接父级逐级冒泡；不得等待整棵后代子树 settled。

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
