## Context

当前系统里已经存在以下零件：

- `kernel::agent_tree` 中的 parent delivery queue 与 batch checkout / requeue / consume
- `application::agent::wake` 中的 `reactivate_parent_agent_if_idle`
- `session-runtime` 的 prompt 提交流程

但 child execution 终态没有稳定调用 wake 管线，导致“子代理完成后如何回流父级”停留在半接线状态。这样既违背已有 specs，也会让四工具模型在复杂编排下失去闭环。

## Goals / Non-Goals

**Goals:**

- 恢复 child terminal notification 到 parent wake 的真实调用链
- 让父级 delivery queue、wake prompt、consume/requeue 形成完整闭环
- 明确 child completion、delivery 持久化与 wake 调度分别归属哪个层级
- 让繁忙父级与失败重试路径具有稳定、可观测的行为

**Non-Goals:**

- 不重做整个 mailbox 或 event model
- 不在本 change 中处理 profile 解析与 root/subagent 执行入口收口
- 不引入新的 transport 或 UI 协议

## Decisions

### 决策 1：child terminal notification 在 `application` 执行编排完成点产生

child 执行的终态摘要、最终回复摘录与 parent delivery 所需的 `ChildSessionNotification` 由 `application` 在 child execution finalize 阶段统一生成。

原因：

- `session-runtime` 负责单 session 真相，不应知道父级 agent 协作语义
- `application` 已经持有 child/parent 编排上下文，更适合产出跨会话协作事件

备选方案：

- 在 `session-runtime` turn 终态直接生成 parent delivery  
  不采用，因为这会把跨会话协作逻辑重新塞回单会话真相层

### 决策 2：wake service 继续留在 `application`，由 child completion 主动调用

不把 wake 逻辑下沉到 kernel 或 session-runtime，而是保留 `application::agent::wake` 作为唯一调度入口，并从 child completion 显式调用它。

原因：

- wake 本质上是跨会话、跨 agent 的业务编排，不是 kernel 寻址或 session-runtime 本地执行问题
- 现有实现已经在 `application`，补接线比重新分层更稳

### 决策 3：父级 delivery queue 继续由 kernel 持有，`application` 只通过稳定控制合同访问

parent delivery 的排队、批次 checkout、requeue、consume 继续放在 kernel 控制平面，`application` 只调用稳定接口。

原因：

- queue 是全局控制状态，属于 kernel 的职责
- 这样可以避免 application 保存额外 shadow state

### 决策 4：父级 wake 失败采用“显式 requeue + 指标记录”，不做静默吞掉

当父级繁忙或 wake turn 提交失败时：

- 繁忙：保留队列，等待后续 drain
- 提交失败：显式 requeue 批次
- 两类路径都更新日志与 observability

原因：

- 这符合“重要失败必须显式暴露”的项目原则
- 能避免 child 结果丢失或被错误消费

## Risks / Trade-offs

- [Risk] child completion 与 wake 调度混在一个 finalize 流程中，可能放大失败表面  
  → Mitigation：保持 notification 生成、queue 入队、wake 调度三个步骤的错误边界清晰分离

- [Risk] 父级繁忙时若只依赖当前触发时机，队列可能滞留  
  → Mitigation：在父 turn 完成后继续 drain parent delivery queue，保持后续补偿触发

- [Risk] notification 摘要与最终回复摘录的来源不一致，可能导致 UI 和 wake prompt 文案不一致  
  → Mitigation：定义单一 notification 构造函数，统一供 UI 投影与 wake prompt 复用

## Migration Plan

1. 补 child finalize → notification → wake service 的正式调用链
2. 统一 delivery queue 的 consume / requeue 语义
3. 为 busy / failed / repeated wake 场景补测试
4. 再对 observability 指标做一致性校验

回滚策略：

- 若自动 wake 在集成环境中出现重复唤醒，可临时保留 notification + queue，但关闭自动调度，作为降级模式

## Open Questions

- 父级 wake prompt 是否需要进一步与 prompt declaration 体系合流；本 change 先保持现有文本构造策略
