# terminal-chat-read-model Specification

## Purpose
定义终端与其他交互式 surface 共享的 conversation hydration / stream read model 合同。

## Requirements

### Requirement: 终端读模型 SHALL 提供无需客户端重放 reducer 的 hydration snapshot

服务端 MUST 为终端客户端提供可直接渲染的 hydration snapshot，用于恢复当前 session 的 transcript、执行状态、待处理控制状态与 child 摘要；终端客户端 MUST NOT 依赖重放原始 agent events 或复制图形前端 reducer 才能进入可用界面。

#### Scenario: 打开 session 时进行 hydration

- **WHEN** 终端客户端首次打开一个存在的 session
- **THEN** 服务端 SHALL 返回足以渲染当前终端界面的 hydration snapshot
- **AND** 客户端 MUST 能在不本地回放完整历史事件的前提下展示当前状态

#### Scenario: 重连后重新 hydration

- **WHEN** 终端客户端在 SSE 中断、进程重启或会话切换后重新连接同一个 session
- **THEN** 服务端 SHALL 允许客户端重新获取该 session 的最新 hydration snapshot
- **AND** snapshot MUST 反映最新服务端事实而不是旧客户端缓存

#### Scenario: 请求不存在的 session snapshot

- **WHEN** 终端客户端请求一个不存在或无权限的 session snapshot
- **THEN** 服务端 SHALL 返回明确的错误结果
- **AND** MUST NOT 返回看似成功但内容为空的假 snapshot

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

### Requirement: 终端增量流 SHALL 支持有序 catch-up 与可恢复消费

终端专属增量流 MUST 为客户端提供有序、可恢复的 delta 消费语义。客户端可基于 cursor / sequence 继续消费后续更新；若 cursor 已失效，服务端 MUST 明确要求重新 hydration，而不是让客户端猜测缺失区间。

#### Scenario: 正常消费实时增量

- **WHEN** 终端客户端已完成 hydration 且持续订阅终端增量流
- **THEN** 服务端 SHALL 以有序 delta 推送后续 transcript、control state 与 child 状态变化
- **AND** 客户端 MUST 能仅凭这些 delta 更新当前界面

#### Scenario: 使用 cursor 进行 catch-up

- **WHEN** 客户端携带上次已消费的 cursor / sequence 重新订阅同一个 session
- **THEN** 服务端 SHALL 从该位置之后继续返回缺失增量
- **AND** MUST NOT 要求客户端回放整个 session 历史

#### Scenario: cursor 失效时回退到 rehydrate

- **WHEN** 客户端携带的 cursor 已过旧、所属 session 已被压缩到无法安全补发，或服务端无法确认缺失区间
- **THEN** 服务端 SHALL 明确拒绝继续增量补发并要求客户端重新获取 hydration snapshot
- **AND** MUST NOT 发送可能导致终端状态错乱的部分增量

### Requirement: conversation surface SHALL 成为终端前端的 authoritative read surface

`conversation` surface 的 snapshot 与 stream MUST 是终端前端消费的 authoritative hydration / delta 合同。旧 `/view`、`/history` 与 `/events` 可以继续存在，但不得再被定义为终端前端的 hydration 或 live delta 来源。工具展示 contract MUST 由 `conversation` surface 直接暴露，客户端 MUST NOT 在本地把多个低层 block 或 replay/event 语义重新组合成工具展示结构。

#### Scenario: 终端前端使用 conversation snapshot 进行 hydration

- **WHEN** 终端客户端进入某个 session
- **THEN** 它 SHALL 使用 conversation snapshot 作为 authoritative hydration 来源
- **AND** MUST NOT 依赖已删除的 `/view` 或 `/history` 来重建 terminal 初始状态

#### Scenario: 终端前端使用 conversation stream 消费增量

- **WHEN** 终端客户端需要消费 live delta
- **THEN** 它 SHALL 订阅 conversation surface 暴露的专属 stream
- **AND** MUST NOT 把已删除的 `/events` 解释为 terminal block 语义

#### Scenario: 工具展示直接消费 authoritative tool block

- **WHEN** 客户端渲染某个 tool call 的 summary、stdout/stderr、失败态或子会话关联
- **THEN** 它 SHALL 直接消费 conversation surface 返回的工具展示结构
- **AND** MUST NOT 在本地执行相邻 regroup、tool stream 扫描或 metadata fallback 推断

### Requirement: 终端读模型 SHALL 提供会话导航与 child 摘要投影

终端读模型 MUST 提供面向 `/resume` 与 child pane 的稳定导航投影，包括 session 候选、排序/搜索所需字段，以及当前 session 可观察 child 的状态摘要；这些投影 MUST 来自现有服务端事实源，而不是终端客户端私有索引。

#### Scenario: 查询恢复候选

- **WHEN** 终端客户端请求 `/resume` 所需的会话候选
- **THEN** 服务端 SHALL 返回带有 session id、标题、最近活动信息及搜索所需字段的稳定投影
- **AND** 这些候选 MUST 能支持终端按关键字筛选与最近使用排序

#### Scenario: 查询 child 摘要

- **WHEN** 终端客户端请求当前 session 的 child agent / subagent 摘要
- **THEN** 服务端 SHALL 返回 direct child 的标识、状态、最近输出摘要与父子关系信息
- **AND** MUST 只暴露当前 session 有权观察到的 child
