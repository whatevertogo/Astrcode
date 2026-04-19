## ADDED Requirements

### Requirement: turn loop SHALL 不因后台任务或终端会话而进入挂起状态
`session-runtime` MUST 保持 turn 的短生命周期语义；后台任务与终端会话不得通过 suspended turn 表达，而应通过独立事件、通知和后续新输入推进。

#### Scenario: 背景工具启动后 turn 正常结束
- **WHEN** `shell(background)` 成功创建后台任务
- **THEN** `session-runtime` MUST 将当前 tool call 视为已完成的即时结果
- **AND** 当前 turn MUST 可正常结束
- **AND** session phase MUST 不新增 waiting_tool

#### Scenario: 后台任务完成后触发新输入
- **WHEN** 后台任务完成或失败
- **THEN** `session-runtime` MUST 记录该完成事实
- **AND** 若策略要求继续让模型处理结果，系统 MUST 通过内部 queued input 或等价消息触发新的 turn

### Requirement: 后台任务与终端会话 SHALL 以 durable 事件和查询投影为真相
后台任务状态、终端会话状态与完成通知 MUST 进入 durable 事件流，并由 query / replay 投影恢复，而不是只存在于内存态。

#### Scenario: 重放后台任务历史
- **WHEN** 系统回放一个包含后台任务历史的 session
- **THEN** query 层 MUST 能恢复出任务 started / completed / failed / lost 的事实
- **AND** 客户端 hydration MUST 能看到对应后台任务通知块或任务摘要

#### Scenario: 丢失 live handle 后投影失败终态
- **WHEN** durable 历史显示某个后台任务或终端会话仍在运行，但 live 侧已确认底层进程不可恢复
- **THEN** `session-runtime` MUST 追加显式失败或 lost 事实
- **AND** query MUST 不再无限投影其为仍可继续运行

### Requirement: session-runtime SHALL 区分工具块、后台任务通知块与终端会话块
`session-runtime` 的 authoritative conversation read model MUST 把“一次性工具调用”“后台任务通知”和“持久终端会话”建模为不同 block 类型，并分别使用稳定主键。

#### Scenario: 背景 shell 的启动结果属于 tool call，完成结果属于后台任务通知
- **WHEN** `shell` 以后台模式执行
- **THEN** 原始 tool call block 只表达“任务已启动”的即时结果
- **AND** 后续完成或失败信息 MUST 进入后台任务通知块
- **AND** MUST NOT 被投影成 terminal session block

#### Scenario: 持久终端会话拥有独立 block
- **WHEN** 模型创建一个持久终端会话并多次向其输入
- **THEN** `session-runtime` MUST 为该会话维护独立 terminal session block
- **AND** 后续输出、stdin 交互记录与状态变化 MUST 持续 patch 该 block
