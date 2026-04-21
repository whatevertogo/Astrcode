## ADDED Requirements

### Requirement: `application` SHALL expose terminal session surface through app-owned contracts

`application` MUST 为 terminal / conversation surface 定义自己的稳定合同，并通过这些合同向 `server` 暴露 conversation snapshot、stream replay、rehydrate、control state、child summaries 与 slash candidates。`server` SHALL 只消费这些 application-owned contracts，SHALL NOT 继续直接依赖 runtime `Conversation*Facts`。

terminal / conversation 合同面至少 SHALL 覆盖：

- block
- delta
- patch
- status
- snapshot
- replay
- rehydrate
- authoritative summary 所需的 control / child / slash summaries

这些 contract 可以按模块拆分，但 `TerminalFacts.transcript` 与 `TerminalStreamReplayFacts.replay` 对外暴露的字段 MUST 属于 `application` 自己的类型，而不是 runtime snapshot / replay 类型别名。

#### Scenario: conversation snapshot 通过 application-owned facts 返回
- **WHEN** `server` 请求某个 session 的 conversation hydration snapshot
- **THEN** `application` SHALL 返回自身定义的 terminal / conversation snapshot contracts
- **AND** `server` SHALL NOT 直接处理 runtime `ConversationSnapshotFacts`

#### Scenario: terminal facts 不再直接承载 runtime transcript
- **WHEN** 检查 `application` 暴露给 `server` 的 `TerminalFacts`
- **THEN** `transcript` 字段 SHALL 是 application-owned snapshot contract
- **AND** SHALL NOT 直接使用 runtime `ConversationSnapshotFacts`

#### Scenario: conversation stream replay 通过 application-owned facts 返回
- **WHEN** `server` 请求某个 session 的 conversation stream replay 或 rehydrate 结果
- **THEN** `application` SHALL 返回自身定义的 replay / delta / rehydrate contracts
- **AND** `server` SHALL NOT 直接处理 runtime `ConversationStreamReplayFacts`

#### Scenario: terminal stream replay 不再直接承载 runtime replay
- **WHEN** 检查 `application` 暴露给 `server` 的 `TerminalStreamReplayFacts`
- **THEN** `replay` 字段 SHALL 是 application-owned replay contract
- **AND** SHALL NOT 直接使用 runtime `ConversationStreamReplayFacts`

#### Scenario: terminal surface contracts 保持纯数据
- **WHEN** 检查 `application` 暴露给 `server` 的 terminal / conversation surface 类型
- **THEN** 这些类型 SHALL 只包含纯数据字段
- **AND** SHALL NOT 直接承载 runtime projector、锁、channel handle 或其他运行时内部对象

### Requirement: `application` SHALL own stream projection coordination for terminal delta consumption

conversation stream 的 authoritative summary、catch-up replay 与 live delta projection MUST 由 `application` 拥有。`server` MAY 负责 SSE 订阅循环和 framing，但 SHALL NOT 直接实例化 runtime `ConversationStreamProjector` 或继续持有 runtime 专属 projection 状态。

#### Scenario: server 不再直接实例化 runtime stream projector
- **WHEN** `server` 处理 conversation SSE 路由
- **THEN** 它 SHALL 通过 `application` 暴露的 stream projection surface 获取 delta
- **AND** SHALL NOT 直接创建 runtime `ConversationStreamProjector`

#### Scenario: application 持有 projection 协调状态但不重写 runtime 算法
- **WHEN** `application` 为 conversation stream 暴露 projection coordination
- **THEN** 该协调状态 SHALL 归属于 `application`
- **AND** 内部 MAY 继续使用 runtime `ConversationStreamProjector`
- **AND** `server` SHALL 只消费 application 暴露的 replay / durable / live / recover surface

#### Scenario: authoritative summary 的合并逻辑留在 application
- **WHEN** 对话流需要根据 control state、child summaries 与 slash candidates 生成附加 delta
- **THEN** 这些 authoritative summary 的比较与合并 SHALL 由 `application` 负责
- **AND** `server` SHALL 只负责把结果映射成 protocol DTO

### Requirement: `application` SHALL own session creation validation at the server boundary

`server -> application` 边界上的 session create 输入校验 MUST 由 `application` use case 拥有。`server` MAY 做空值与 JSON 形状校验，但 SHALL NOT 直接调用 runtime `normalize_working_dir` 或等价路径 helper。

#### Scenario: create session route 不直接调用 runtime working-dir helper
- **WHEN** `server` 处理创建 session 的 HTTP 请求
- **THEN** 工作目录规范化与合法性校验 SHALL 由 `application` use case 或其 port 实现处理
- **AND** route 层 SHALL NOT 直接调用 runtime 路径 helper

#### Scenario: 非法 working directory 通过 application error 返回
- **WHEN** 用户提交不存在、非法或不是目录的 `workingDir`
- **THEN** `application` SHALL 返回明确的业务错误
- **AND** `server` 只负责把该错误映射成 HTTP 响应

### Requirement: `application` SHALL hide runtime fork result behind app-owned fork surface

`server -> application` 的 fork 输入 MUST 使用 application-owned selector，而 runtime `ForkPoint` 与 `ForkResult` SHALL 留在 application port / session-runtime 内部。`App::fork_session()` 对 `server` 的稳定返回值 SHALL 是 `SessionMeta`。

#### Scenario: App::fork_session 不向 server 暴露 runtime ForkResult
- **WHEN** `server` 调用 `App::fork_session`
- **THEN** 它 SHALL 收到 `SessionMeta`
- **AND** SHALL NOT 观察 runtime `ForkResult` 的字段结构
