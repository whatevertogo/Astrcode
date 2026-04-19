## ADDED Requirements

### Requirement: conversation read model SHALL 暴露后台任务通知块
authoritative conversation read model MUST 为后台任务提供正式通知块，使客户端能够区分“任务已启动”“任务已完成”“任务已失败”“任务已丢失”，而不是通过工具 metadata 或文本摘要猜测。

#### Scenario: hydration 返回后台任务历史
- **WHEN** 客户端为一个包含后台任务历史的 session 请求 hydration snapshot
- **THEN** 服务端 MUST 返回后台任务启动与终态通知块
- **AND** 客户端 MUST 无需回放原始事件即可识别任务是否已经完成

#### Scenario: 后台任务完成生成独立通知
- **WHEN** 某个后台任务最终完成、失败或取消
- **THEN** 增量流 MUST 追加一条独立的后台任务终态通知
- **AND** 该通知 MUST 包含 `backgroundTaskId`、终态和输出引用

### Requirement: conversation read model SHALL 暴露 terminal session block
authoritative conversation read model MUST 为持久终端会话提供独立 block 类型，至少包含 `terminal_session_id`、状态、stdout/stderr 聚合结果、cursor 以及增量 patch 语义。

#### Scenario: terminal session 首次启动进入 transcript
- **WHEN** 系统创建一个新的终端会话
- **THEN** hydration / delta 流 MUST 追加一个新的 terminal session block
- **AND** 该 block MUST 暴露稳定的 `terminal_session_id`

#### Scenario: terminal session 输出持续 patch 同一 block
- **WHEN** 终端会话持续产出 stdout 或 stderr
- **THEN** conversation delta MUST 以 append patch 更新同一 terminal session block
- **AND** 客户端 MUST NOT 自行把多次 tool result 重新拼装成终端视图

#### Scenario: terminal session 状态变化进入同一 block
- **WHEN** 终端会话进入 running、waiting-input、closed、failed、lost 或 exited
- **THEN** conversation delta MUST 在同一 terminal session block 上更新状态
- **AND** 客户端 MUST 能在不重建 transcript 的情况下显示最新状态
