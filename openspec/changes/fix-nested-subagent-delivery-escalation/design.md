## 设计概览

本次修复采用“每轮独立收口、直接父级逐级冒泡”的模型。

### 决策 1：child turn terminal finalizer 统一收口

所有 child turn 在进入 terminal 状态后，都进入同一个 application 级 finalizer：

1. 等待 `session_id + turn_id` 的 terminal snapshot
2. 投影 `AgentTurnOutcome / summary / final_reply_excerpt`
3. 基于显式 parent routing context 构造 `ChildSessionNotification`
4. 向父侧 session 追加 durable notification
5. 触发直接父级 wake

这样 spawn、resume、wake 不再各自维护一套终态逻辑。

### 决策 2：异常终态仍然向上级投递

如果 child turn 的业务终态是 `Failed / Cancelled / TokenExceeded`，它仍然属于 terminal turn，因此仍要生成 terminal delivery。

这与“finalizer 自身失败”不同：

- 业务终态失败：要正式上报
- finalizer 自身失败：不得把当前批次标记为成功消费

### 决策 3：禁止从 `ChildAgentRef` 反推路由

内部实现不再通过 `child_ref.session_id` 推导“父侧 notification 写到哪个 session”。

路由真相由 finalizer context 显式携带：

- `parent_session_id`
- `parent_turn_id`
- `parent_agent_id`
- `execution_session_id`
- `execution_turn_id`

`ChildAgentRef` 继续承担 stable child reference / projection 角色，但不再承担 parent routing truth。

### 决策 4：wake turn 在 batch 收口前先做上行 finalization

对于由 delivery 触发的 wake turn：

- 先等待当前 turn terminal
- 如果当前执行者本身是 child agent，则先尝试对它的直接父级发布 terminal delivery
- 上行 finalization 成功后，再执行当前 batch 的 `acked / consume`
- 如果上行 finalization 失败，则当前 batch 保持可 requeue，避免“叶子结果被消费但更上级永远收不到”

### 决策 5：turn 完成不等待后代 settled

“middle 的 turn 完成”只表示 `middle` 当前这一轮结束。

如果 `middle` 在这一轮里又产生了新的 child work：

- `middle` 当前 turn 仍应立即向 `root` 汇报
- 新 child 的完成由后续独立 delivery/wake 周期继续回传

不引入树级等待语义，避免生命周期耦合、循环等待和重复汇报。
