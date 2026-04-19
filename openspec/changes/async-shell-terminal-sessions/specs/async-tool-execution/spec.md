## ADDED Requirements

### Requirement: 长耗时工具调用 SHALL 支持立即转为后台任务
当工具执行满足后台化条件时，系统 MUST 允许该工具调用立即返回后台任务信息，而不是持续阻塞同一个 turn loop 直到工具返回最终结果。

#### Scenario: shell 命令显式请求后台执行
- **WHEN** 模型调用 `shell` 且参数声明 `executionMode=background`
- **THEN** 系统 MUST 为该调用创建后台任务
- **AND** 当前 tool result MUST 返回 `backgroundTaskId`
- **AND** 当前 turn MUST 正常结束，而不是进入挂起状态

#### Scenario: 自动策略判定转入后台执行
- **WHEN** 工具执行达到后台化策略阈值，且该工具声明允许后台化执行
- **THEN** 系统 MUST 将该调用转换为后台任务
- **AND** MUST 返回可读取输出的稳定路径或等价句柄

### Requirement: 后台执行接入 SHALL 通过稳定进程监管端口
支持后台任务或持久终端会话的工具 MUST 通过 `core` 定义的稳定进程监管端口接入运行时控制面；`adapter-tools` MUST NOT 直接依赖 `application` concrete type，也 MUST NOT 自行维护未受监管的后台子进程真相。

#### Scenario: shell 通过注入端口启动后台任务
- **WHEN** builtin `shell` 需要切换到后台执行
- **THEN** 它 MUST 通过 `ToolContext` / `CapabilityContext` 注入的进程监管端口注册任务
- **AND** `application` MUST 作为该端口的实现拥有 live handle 真相
- **AND** 具体 PTY / process driver MUST 仍由组合根在 adapter 边界绑定

### Requirement: 自动后台化判定 SHALL 可审计且跨层一致
`executionMode=auto` 的解析 MUST 由统一策略完成，而不是散落在工具实现内；即时 tool result 与 durable 事件 MUST 暴露请求模式、最终模式与判定原因，保证前端展示、排障与回放看到同一语义。

#### Scenario: auto 模式被解析为 background
- **WHEN** 模型调用 `shell` 且参数声明 `executionMode=auto`
- **THEN** 系统 MUST 记录 `requestedExecutionMode=auto`
- **AND** MUST 在即时结果与后续 started 事实中暴露 `resolvedExecutionMode`
- **AND** 若最终进入后台，MUST 追加可读的判定原因或策略来源

### Requirement: 后台工具输出 SHALL 进入稳定输出存储并可被显式读取
后台工具在执行期间产生的 stdout / stderr MUST 持续写入稳定输出存储，并允许后续通过输出路径或正式读取能力获取。

#### Scenario: 后台 shell 持续写入输出文件
- **WHEN** 已进入后台的 shell 任务继续产生 stdout 或 stderr
- **THEN** 系统 MUST 将这些输出持续写入与 `backgroundTaskId` 关联的稳定输出存储
- **AND** 该存储路径或读取句柄 MUST 对模型可见

#### Scenario: 后台工具最终完成
- **WHEN** 后台工具成功完成
- **THEN** 系统 MUST 产出独立的 completed 通知事件
- **AND** 通知中 MUST 包含 `backgroundTaskId`、总结信息与输出存储引用
- **AND** 系统 MAY 额外注入一条内部输入以唤醒后续新 turn

### Requirement: 后台工具失败、取消与丢失 SHALL 有显式终态
后台工具执行不得以“没有后续输出”来表达失败或丢失；系统 MUST 产出明确的失败、取消或 lost 终态，并结束对应后台任务。

#### Scenario: 用户取消等待中的后台工具
- **WHEN** 用户或系统取消一个运行中的后台工具
- **THEN** 系统 MUST 终止底层任务或标记其为已取消
- **AND** 对应后台任务 MUST 进入 cancelled 或 failed 终态
- **AND** 系统 MUST 发送明确的取消通知

#### Scenario: Astrcode 进程重启后丢失后台任务句柄
- **WHEN** 系统重启后发现某个后台任务的 live handle 已不可恢复
- **THEN** 系统 MUST 将该后台任务显式标记为 lost 或 failed
- **AND** MUST NOT 无限保留一个看似仍可完成的 running 状态
