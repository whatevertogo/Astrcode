## ADDED Requirements

### Requirement: 系统 SHALL 提供持久终端会话工具合同
系统 MUST 提供与一次性 `shell` 分离的持久终端会话能力，使模型可以创建一个终端会话并获得稳定的 `terminal_session_id`，后续所有输入输出与状态更新 MUST 归属于该标识。

#### Scenario: 创建新的终端会话
- **WHEN** 模型调用 `terminal_start`
- **THEN** 系统 MUST 启动一个新的终端会话并返回稳定的 `terminal_session_id`
- **AND** transcript / conversation read model MUST 追加对应的 terminal session block

#### Scenario: 同一会话跨多次输入保持同一身份
- **WHEN** 模型随后多次调用 `terminal_write`
- **THEN** 每次输入 MUST 绑定到已有的 `terminal_session_id`
- **AND** 系统 MUST NOT 为每次输入新建一个独立终端会话实体

### Requirement: 终端会话 SHALL 支持显式 stdin 写入与显式输出读取
终端会话 MUST 支持向底层 shell / PTY 写入 stdin，并通过正式读取接口暴露自某个 cursor 之后的新 stdout / stderr 输出。

#### Scenario: 模型写入终端输入
- **WHEN** 模型调用 `terminal_write` 并携带文本输入
- **THEN** 系统 MUST 将该文本写入对应终端会话的 stdin
- **AND** 该调用本身 MUST 返回明确的接受结果，而不是假定输入已经执行完毕

#### Scenario: 读取自某个 cursor 之后的新输出
- **WHEN** 模型调用 `terminal_read` 并携带某个 `cursor`
- **THEN** 系统 MUST 返回该 cursor 之后产生的新 stdout / stderr 输出片段
- **AND** MUST 返回新的 cursor 供后续继续读取
- **AND** 客户端或模型 MUST 无需重新读取整个终端历史

### Requirement: 终端会话读取 SHALL 支持有限等待策略
终端读取工具 MAY 支持受限的长轮询读取，但系统 MUST 不以 suspended turn 或隐藏等待状态表达该能力。

#### Scenario: 仅发送输入立即返回
- **WHEN** 模型调用 `terminal_write`
- **THEN** 系统 MUST 在写入 stdin 后立即返回
- **AND** 当前 turn MUST NOT 进入挂起状态

#### Scenario: 读取时使用短暂长轮询
- **WHEN** 模型调用 `terminal_read` 且声明短暂等待窗口
- **THEN** 系统 MAY 在该窗口内等待新输出到达
- **AND** 超时后 MUST 返回当前已知输出和最新 cursor
- **AND** MUST NOT 把该读取升级为长期挂起的 turn 状态

### Requirement: 终端会话 SHALL 支持显式关闭与明确失败语义
系统 MUST 允许模型显式关闭终端会话；若终端进程崩溃、会话丢失或系统重启导致无法继续控制，也 MUST 产生明确的失败或 lost 终态。

#### Scenario: 模型关闭终端会话
- **WHEN** 模型调用 `terminal_close`
- **THEN** 系统 MUST 关闭对应终端会话并追加 closed 终态
- **AND** 后续对该 `terminal_session_id` 的输入 MUST 被拒绝

#### Scenario: 终端进程异常退出
- **WHEN** 底层终端进程非预期退出
- **THEN** 系统 MUST 将 terminal session 标记为 failed 或 exited
- **AND** 会话读模型 MUST 暴露退出码或失败原因

#### Scenario: 系统重启后会话句柄丢失
- **WHEN** Astrcode 重启且无法重新附着到某个终端会话的底层进程
- **THEN** 系统 MUST 将该终端会话标记为 lost 或 failed
- **AND** MUST NOT 继续宣称其仍可输入
