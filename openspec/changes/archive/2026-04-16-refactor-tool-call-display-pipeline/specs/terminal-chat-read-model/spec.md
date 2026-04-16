## MODIFIED Requirements

### Requirement: 终端 transcript SHALL 以结构化 block 暴露稳定语义

终端读模型 MUST 把 transcript 暴露为带稳定标识和显式类型的结构化 block，而不是要求客户端自己把细粒度 agent events 重新拼成 UI。block 类型至少 MUST 覆盖 user message、assistant message、thinking、tool call、turn-scoped error、compact / system note 与 child handoff。工具展示相关语义 MUST 以后端聚合后的 tool call block 作为唯一主实体；stdout、stderr、终态、错误、duration、truncated 与子会话关联 MUST 作为该实体的稳定字段或稳定 patch 语义暴露，客户端 MUST NOT 依赖相邻 block regroup、原始 event 重放或 metadata 猜测来恢复工具展示真相。

#### Scenario: thinking 与 assistant 内容属于不同 block 语义

- **WHEN** 一次回复同时包含 thinking 过程与最终 assistant 内容
- **THEN** 读模型 SHALL 将两者表示为可区分的结构化 block
- **AND** 客户端 MUST 能在不解析原始事件细节的情况下稳定渲染它们

#### Scenario: 工具输出增量补丁归属于同一 tool call block

- **WHEN** 一个 tool call 在执行期间多次产出 stdout、stderr 或状态更新
- **THEN** 读模型 SHALL 以稳定 `tool_call_id` 把这些增量归属于同一 tool call block
- **AND** 客户端 MUST 能仅通过该 block 的 patch 更新完成工具展示
- **AND** MUST NOT 依赖相邻 transcript block 顺序来推断归属关系

#### Scenario: tool call block 暴露完整终态字段

- **WHEN** 某个 tool call 完成、失败或被截断
- **THEN** 读模型 SHALL 在同一 tool call block 中暴露明确终态、错误、duration 与 truncated 语义
- **AND** 客户端 MUST NOT 通过解析文本 summary 或额外本地推断来区分这些状态

#### Scenario: 子智能体交接进入 transcript

- **WHEN** root session 向 child agent / subagent 委派任务或接收 child terminal result
- **THEN** 读模型 SHALL 在 transcript 中生成可识别的 child handoff / child result block
- **AND** 该 block MUST 能关联到对应的 child 摘要视图

#### Scenario: turn 级错误进入 transcript

- **WHEN** 当前 turn 产生 provider error、context window exceeded、tool fatal 或本轮 rate limit 错误
- **THEN** 读模型 SHALL 生成可识别的 error block
- **AND** 该错误 MUST 与所属 turn 语义关联，而不是只作为 transport 层错误文本暴露

#### Scenario: 连接级错误不进入 transcript

- **WHEN** 客户端遇到 `auth_expired`、`cursor_expired` 或 `stream_disconnected`
- **THEN** 读模型 MUST NOT 伪造 transcript error block
- **AND** 客户端 SHALL 通过 banner、status 或重连状态处理这些错误

### Requirement: conversation surface SHALL 成为终端前端的 authoritative read surface

`conversation` surface 的 snapshot 与 stream MUST 是终端前端消费的 authoritative hydration / delta 合同。旧 `/view`、`/history` 与 `/events` 可以继续存在，但不得再被定义为终端前端的 hydration 或 live delta 来源。工具展示 contract MUST 由 `conversation` surface 直接暴露，客户端 MUST NOT 在本地把多个低层 block 或 replay/event 语义重新组合成工具展示结构。

#### Scenario: 终端前端使用 conversation snapshot 进行 hydration

- **WHEN** 终端客户端进入某个 session
- **THEN** 它 SHALL 使用 conversation snapshot 作为 authoritative hydration 来源
- **AND** MUST NOT 依赖 legacy `/view` 或 `/history` 来重建 terminal 初始状态

#### Scenario: 终端前端使用 conversation stream 消费增量

- **WHEN** 终端客户端需要消费 live delta
- **THEN** 它 SHALL 订阅 conversation surface 暴露的专属 stream
- **AND** MUST NOT 把 legacy `/events` 解释为 terminal block 语义

#### Scenario: 工具展示直接消费 authoritative tool block

- **WHEN** 客户端渲染某个 tool call 的 summary、stdout/stderr、失败态或子会话关联
- **THEN** 它 SHALL 直接消费 conversation surface 返回的工具展示结构
- **AND** MUST NOT 在本地执行相邻 regroup、tool stream 扫描或 metadata fallback 推断
