## Purpose

定义 child delivery 的稳定控制合同，包括父级交付、唤醒、关闭和观察的完整生命周期，以及父子协作交付的逐级冒泡语义。

## Core Types

### ParentDelivery（core 层）

- `idempotency_key`: String — 幂等键
- `origin`: ParentDeliveryOrigin — `Explicit` | `Fallback`
- `terminal_semantics`: ParentDeliveryTerminalSemantics — `Terminal` | `NonTerminal`
- `source_turn_id`: Option<String> — 来源 child work turn
- `payload`: ParentDeliveryPayload — 判别联合 payload

### ParentDeliveryPayload（core 层，tag = "kind"）

- `Progress(ProgressParentDeliveryPayload { message })`
- `Completed(CompletedParentDeliveryPayload { message, findings, artifacts })`
- `Failed(FailedParentDeliveryPayload { message, code: SubRunFailureCode, technical_message, retryable })`
- `CloseRequest(CloseRequestParentDeliveryPayload { message, reason })`

方法：
- `kind() -> ParentDeliveryKind`
- `message() -> &str`

### ChildSessionNotification（core 层）

- `notification_id`: DeliveryId
- `child_ref`: ChildAgentRef
- `kind`: ChildSessionNotificationKind
- `source_tool_call_id`: Option<DeliveryId>
- `delivery`: Option<ParentDelivery>

### SubRunHandoff（core 层）

- `findings`: Vec<String>
- `artifacts`: Vec<ArtifactRef>
- `delivery`: Option<ParentDelivery>

### SendToChildParams（core 层）

- `agent_id`: AgentId — 目标子 Agent 的稳定 ID
- `message`: String — 追加给子 Agent 的消息内容
- `context`: Option<String> — 可选补充上下文

方法：
- `validate()` — 校验 agent_id 和 message 非空

### SendToParentParams（core 层）

- `payload`: ParentDeliveryPayload（flatten）

方法：
- `validate()` — 校验 message 非空

## Requirements

### Requirement: Stable Agent Delivery And Control Contract

系统 SHALL 为 child delivery、observe、wake、close 暴露稳定控制合同，并在 child 终态出现时通过正式 delivery queue 与 parent wake 管线驱动父级后续执行。child -> parent 回流 MUST 以 typed parent-delivery message 表达，而不是依赖 `summary` 或 server summary projection。

#### Scenario: Deliver message to child agent

- **WHEN** 上层向某个 child agent 或 subrun 路由消息
- **THEN** 系统 SHALL 通过稳定控制接口（`KernelAgentSurface::deliver`）完成投递
- **AND** 调用方 SHALL 不需要了解内部 input queue 实现

#### Scenario: Child delivers typed message to direct parent through unified send

- **WHEN** child agent 在自己的执行上下文中调用 unified `send`
- **AND** 其输入匹配 upward delivery payload
- **THEN** 系统 SHALL 把该消息持久化为 typed parent-delivery message（`ParentDeliveryOrigin::Explicit`）
- **AND** MUST NOT 依赖 `summary` 或 server 生成的摘要字段表达这条回流事实

#### Scenario: Middle agent keeps both upstream and downstream send edges

- **WHEN** 一个非 root agent 同时存在 direct parent 与 direct child
- **THEN** 系统 MUST 允许它对 direct parent 写 upward delivery
- **AND** MUST 同时允许它继续向 direct child 写 downstream message
- **AND** MUST 仅按目标边和参数分支决定路由，而不是把该 agent 固定成单一角色

#### Scenario: typed upward delivery payload is discriminated by kind

- **WHEN** 系统定义或序列化 typed parent-delivery message
- **THEN** `payload` MUST 按 `kind` 做判别联合（`#[serde(tag = "kind", content = "payload")]`），而不是无结构 blob
- **AND** `completed`、`failed`、`close_request`、`progress` MUST 各自拥有最小字段集

#### Scenario: typed delivery carries turn identity and origin

- **WHEN** 系统写入 typed parent-delivery message
- **THEN** 该消息 MUST 包含稳定 `idempotency_key`
- **AND** MUST 包含 `origin = explicit | fallback`
- **AND** MUST 包含 `terminal_semantics = terminal | non_terminal`
- **AND** MUST 包含当前 child work turn 的 `source_turn_id`

#### Scenario: Wake suspended agent

- **WHEN** 上层请求唤醒可恢复的 agent 或 subrun
- **THEN** 系统 SHALL 通过 `KernelAgentSurface::resume` 执行唤醒
- **AND** 失败 SHALL 明确暴露为领域错误

#### Scenario: Close agent subtree

- **WHEN** 上层请求关闭某个 agent 子树
- **THEN** 系统 SHALL 通过 `close_subtree` 提供统一关闭合同
- **AND** 该合同 SHALL 明确返回 `CloseSubtreeResult`（closed_count + closed_agent_ids）

#### Scenario: Observe agent execution

- **WHEN** 上层订阅某个 agent 或 subrun 的执行过程
- **THEN** 系统 SHALL 通过 `observe_child` 返回 `ObserveSnapshot`（含 lifecycle_status、phase、turn_count、active_task、last_output_tail、last_turn_tail）
- **AND** 内置幂等去重：同一 turn 内连续 observe 相同状态返回 `state_unchanged` 错误
- **AND** SHALL 不暴露内部事件总线协议

#### Scenario: Child completion wakes parent through delivery pipeline

- **WHEN** 子代理完成、失败、请求关闭或被关闭且需要向父级回流结果
- **THEN** 系统 SHALL 先通过 `append_child_session_notification` 持久化 typed parent-delivery
- **AND** 通过 `enqueue_child_delivery` 入队父级交付缓冲
- **AND** 通过 `try_start_parent_delivery_turn` 尝试启动父级后续执行（wake turn）

#### Scenario: Wake failure requeues delivery batch

- **WHEN** 父级 wake 提交失败或父级当前不可获取执行机会
- **THEN** 系统 SHALL 通过 `requeue_parent_delivery_batch` 重新排队对应 delivery batch
- **AND** MUST NOT 静默丢弃 child 终态回流

### Requirement: Parent delivery batch lifecycle

kernel 与 application SHALL 为 parent delivery batch 定义稳定生命周期，使 child 终态回流具备可重试与可观测行为。batch 内的条目 MUST 是 typed parent-delivery message，而不是 summary/excerpt 投影。

#### Scenario: Delivery batch enters waking state

- **WHEN** 系统 `checkout_parent_delivery_batch` 取出一批父级交付用于 wake
- **THEN** 该批次从 kernel buffer 中移除，进入"正在唤醒父级"的中间状态
- **AND** 在被 consume 或 requeue 前不得被重复消费

#### Scenario: Busy parent defers batch consumption

- **WHEN** 父级当前忙碌（submit 返回 None），无法立即开始 wake turn
- **THEN** 该批次通过 `requeue_parent_delivery_batch` 恢复为待重试状态
- **AND** MUST NOT 被提前 consume

#### Scenario: Successful wake consumes batch

- **WHEN** 父级 wake turn 成功接受并完成（Idle + TurnDone + 无 Error）
- **THEN** 系统通过 `consume_parent_delivery_batch` 从 durable 存储中消费该批次
- **AND** 记录 Delivery + Consumed collaboration fact（含 latency_ms）

#### Scenario: Failed wake keeps batch retryable

- **WHEN** 父级 wake turn 提交失败或中途失败
- **THEN** 系统 `requeue_parent_delivery_batch` 重新排队该批次
- **AND** SHALL 记录 `record_parent_reactivation_failed` 指标

#### Scenario: Automatic follow-up limit

- **WHEN** 成功消费后仍有剩余 delivery
- **THEN** 系统自动触发下一轮 follow-up（最大 8 轮）
- **WHEN** 达到上限仍有剩余
- **THEN** 记录 warning log，不再自动触发

### Requirement: Route truth is explicit and rejects invalid upstream sends early

父子协作交付 SHALL 按直接父级逐级冒泡。child 上行时的 direct-parent route truth MUST 来自 durable parent-child 关系，而不是模型输入。parent 不可达时系统 MUST 在 `application` 层前置拒绝并打 log / fact。

#### Scenario: explicit child work turn can still report upward immediately

- **WHEN** `middle` 执行自己的一轮 child work turn
- **AND** 该 turn 在 `leaf` 等后代仍未 settled 时结束
- **THEN** 系统 MUST 仍允许 `middle` 立即向自己的直接父级汇报本轮结果
- **AND** MUST NOT 等待整棵后代子树全部 settled

#### Scenario: invalid upstream send is rejected before queueing

- **WHEN** child 尝试上行发送
- **AND** direct parent 缺失、已关闭、不可达，或调用方并非合法 child
- **THEN** 系统 MUST 在写 durable notification / input queue 之前拒绝该调用
- **AND** MUST 写结构化 log 与 collaboration fact

#### Scenario: explicit upward delivery suppresses duplicate fallback

- **WHEN** child 在当前 child work turn 内已经显式向 direct parent 写入 terminal typed delivery（`origin=Explicit, terminal_semantics=Terminal`）
- **THEN** `finalize_child_turn_with_outcome` 检测到该事实后跳过 fallback delivery
- **AND** MUST NOT 为同一 turn 再生成第二条 terminal upward delivery

#### Scenario: route truth is explicit

- **WHEN** 系统向父侧 session 追加 child upward delivery
- **THEN** 路由落点 MUST 来自显式 `parent_session_id` + `parent_turn_id`
- **AND** MUST NOT 从 `ChildAgentRef.session_id` 反推父侧落点

#### Scenario: middle spawns new child during wake

- **WHEN** `middle` 在处理 wake turn 时又产生新的 child work
- **THEN** 新 child 的完成 SHALL 通过后续独立 delivery/wake 周期继续回传
- **AND** 当前 wake turn MUST NOT 因为自身结束而自动向更上一级制造新的 terminal delivery
- **AND** 系统 MUST NOT 等待整棵后代子树全部 settled 才允许后续显式 child work turn 上报

#### Scenario: wake turn stays at the direct consumer boundary

- **WHEN** `leaf` 的 terminal delivery 唤醒 `middle`
- **AND** `middle` 完成这轮 wake turn
- **THEN** 系统 MUST 在 `middle` 侧完成当前 batch 的消费
- **AND** 新 child 的完成 SHALL 通过后续独立 delivery/wake 周期继续回传
- **AND** MUST NOT 自动继续为 `root` 生成一条新的 child terminal delivery

### Requirement: Child turn terminal watcher

application SHALL 在子代理 turn 结束时启动后台 watcher，执行终态映射、fallback delivery 投影和父级 reactivation。

#### Scenario: watcher 启动

- **WHEN** spawn 成功后，系统调用 `spawn_child_turn_terminal_watcher`
- **THEN** 注册到 `TaskRegistry` 以便 shutdown 时统一 abort

#### Scenario: 终态映射

- **WHEN** child turn 以 `Completed` 或 `TokenExceeded` 结束
- **THEN** 映射为 `SubRunResult::Completed`（TokenExceeded 视为完成而非失败，因为 LLM 通常已输出了有价值的部分结果）
- **WHEN** child turn 以 `Failed` 结束
- **THEN** 映射为 `SubRunResult::Failed`（含 SubRunFailure{ code: Internal, retryable: true }）
- **WHEN** child turn 以 `Cancelled` 结束
- **THEN** 映射为 `SubRunResult::Failed`（含 SubRunFailure{ code: Interrupted, retryable: false }）

#### Scenario: fallback delivery 投影

- **WHEN** child turn 无显式 terminal delivery
- **THEN** 系统投影出 fallback `ChildSessionNotification`（`ParentDeliveryOrigin::Fallback`），payload 类型根据终态选择 Completed/Failed/CloseRequest
