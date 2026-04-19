## ADDED Requirements

### Requirement: 系统 SHALL 提供基于 `process_id` 的持久终端会话工具合同
系统 MUST 提供与一次性 `shell` 分离的持久终端会话能力，使模型可以通过 `exec_command` 创建一个终端会话并获得稳定的 `process_id`，后续所有输入输出与状态更新 MUST 归属于该标识。

#### Scenario: 创建新的终端会话
- **WHEN** 模型调用 `exec_command`
- **THEN** 系统 MUST 启动一个新的终端会话并返回稳定的 `process_id`
- **AND** transcript / conversation read model MUST 追加对应的 terminal session block

#### Scenario: 同一会话跨多次输入保持同一身份
- **WHEN** 模型随后多次调用 `write_stdin`
- **THEN** 每次输入 MUST 绑定到已有的 `process_id`
- **AND** 系统 MUST NOT 为每次输入新建一个独立终端会话实体

### Requirement: 终端会话 SHALL 支持显式 stdin 写入与有限窗口输出返回
终端会话 MUST 支持向底层 shell / PTY 写入 stdin，并在每次 `exec_command` / `write_stdin` 调用中返回该等待窗口内的新 stdout / stderr 输出快照。

#### Scenario: 启动终端时返回初始等待窗口内输出
- **WHEN** 模型调用 `exec_command`
- **THEN** 系统 MUST 在 `yield_time_ms` 指定的有限等待窗口内收集新输出
- **AND** MUST 返回该窗口内的 stdout / stderr 输出快照
- **AND** 若进程仍在运行，响应 MUST 保留 `process_id`

#### Scenario: 模型写入终端输入
- **WHEN** 模型调用 `write_stdin` 并携带文本输入
- **THEN** 系统 MUST 将该文本写入对应终端会话的 stdin
- **AND** 该调用 MUST 返回本次等待窗口内的新输出、当前 `process_id` 和已知退出状态
- **AND** 该调用 MUST NOT 假定输入已经执行完毕

### Requirement: 终端会话交互 SHALL 支持有限等待策略
终端会话工具 MAY 支持受限的短暂等待，但系统 MUST 不以 suspended turn 或隐藏等待状态表达该能力。

#### Scenario: `write_stdin` 使用短暂等待窗口
- **WHEN** 模型调用 `write_stdin` 且声明 `yield_time_ms`
- **THEN** 系统 MAY 在该窗口内等待新输出到达
- **AND** 超时后 MUST 返回当前已知输出与最新进程状态
- **AND** MUST NOT 把该交互升级为长期挂起的 turn 状态

#### Scenario: 空输入用于轮询后台终端输出
- **WHEN** 模型调用 `write_stdin` 且 `chars` 为空字符串
- **THEN** 系统 MAY 将该调用视为一次不写入 stdin 的输出轮询
- **AND** MUST 仍然复用同一 `process_id`
- **AND** MUST 返回等待窗口内观察到的新输出或空输出

### Requirement: 终端会话 SHALL 支持显式控制与明确失败语义
系统 MUST 允许模型显式终止、关闭 stdin 或调整终端尺寸；若终端进程崩溃、会话丢失或系统重启导致无法继续控制，也 MUST 产生明确的失败或 lost 终态。

#### Scenario: 模型终止终端会话
- **WHEN** 模型调用 `terminate_terminal`
- **THEN** 系统 MUST 终止对应终端会话并追加 closed、failed 或 exited 终态
- **AND** 后续对该 `process_id` 的输入 MUST 被拒绝

#### Scenario: 模型关闭 stdin
- **WHEN** 模型调用 `close_stdin`
- **THEN** 系统 MUST 关闭对应终端会话的 stdin
- **AND** 如进程仍存活，系统 MUST 继续保留该会话直到其自然退出或被显式终止

#### Scenario: 模型调整终端尺寸
- **WHEN** 模型调用 `resize_terminal`
- **THEN** 系统 MUST 尝试调整对应 PTY 的终端尺寸
- **AND** 若该会话不是 PTY，会话工具 MUST 返回明确拒绝

#### Scenario: 终端进程异常退出
- **WHEN** 底层终端进程非预期退出
- **THEN** 系统 MUST 将 terminal session 标记为 failed 或 exited
- **AND** 会话读模型 MUST 暴露退出码或失败原因

#### Scenario: 系统重启后会话句柄丢失
- **WHEN** Astrcode 重启且无法重新附着到某个终端会话的底层进程
- **THEN** 系统 MUST 将该终端会话标记为 lost 或 failed
- **AND** MUST NOT 继续宣称其仍可输入

### Requirement: 会话输出主路径 SHALL 通过 durable/live 事件持续投影
终端会话的 stdout / stderr 主通道 MUST 通过 begin/delta/end 事件和终端交互记录持续进入 conversation/read model，而不是要求模型依赖单独的读取游标协议。

#### Scenario: 运行中会话持续发出输出增量
- **WHEN** 持久终端会话持续产出 stdout 或 stderr
- **THEN** 系统 MUST 持续发出与该 `process_id` 关联的输出增量事件
- **AND** conversation read model MUST 把这些输出 patch 到同一 terminal session block

#### Scenario: stdin 写入被记录为交互事件
- **WHEN** 模型对会话调用 `write_stdin`
- **THEN** 系统 MUST 记录对应的 terminal interaction 事件
- **AND** 该交互记录 MUST 能与同一 `process_id` 关联
