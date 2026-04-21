## Purpose

server HTTP 路由层的边界约束：所有 HTTP route、mapper 与 route-local projector 必须通过 `application` 暴露的稳定业务 surface 消费会话能力，不能绕过 application 直接依赖 session-runtime 内部类型。

## Requirements

### Requirement: server HTTP routes SHALL consume business surfaces only through `application`

`server` 的 HTTP route、route mapper 与 route-local projector MUST 通过 `application` 暴露的稳定业务 surface 消费会话能力。除了 bootstrap 组合根与明确的内部 test harness，`server` SHALL NOT 在 HTTP 层直接 import `session-runtime` 的内部 helper、read-model facts、projection state 或 runtime enum。

#### Scenario: terminal projection mapper 不再匹配 runtime conversation facts
- **WHEN** `server` 把 conversation / terminal business facts 映射为 protocol DTO
- **THEN** mapper SHALL 只匹配 `application` 暴露的 terminal contracts
- **AND** SHALL NOT 直接匹配 runtime `ConversationBlockFacts`、`ConversationDeltaFacts` 或等价内部类型

#### Scenario: conversation route 不再直接持有 runtime projector
- **WHEN** `server` 处理 conversation SSE route
- **THEN** route SHALL 通过 `application` 的 stream surface 获取 replay / delta / rehydrate 结果
- **AND** SHALL NOT 直接实例化 runtime `ConversationStreamProjector`

#### Scenario: session mutation route 不再直接使用 runtime helper 与 runtime enum
- **WHEN** `server` 处理 session fork 或 create session 相关 route
- **THEN** route SHALL 通过 `application` 用例完成 fork selector 解析与 working-dir 校验
- **AND** SHALL NOT 直接使用 runtime `ForkPoint` 或 `normalize_working_dir`

#### Scenario: bootstrap 仍可保留 runtime 直连
- **WHEN** `server` 在 bootstrap 组合根中组装 `application`、`kernel`、`session-runtime` 与 adapters
- **THEN** bootstrap MAY 继续直接引用 runtime crate
- **AND** 该例外 SHALL NOT 扩散到 HTTP 路由与 DTO mapper

#### Scenario: HTTP 层实现达到零 runtime import
- **WHEN** 审查 `crates/server/src/http/**` 的实现
- **THEN** 其中 SHALL NOT 直接 import `astrcode_session_runtime`
- **AND** terminal projection、conversation route、session mutation route 与 session route helpers SHALL 只依赖 `application`、`protocol` 与 transport 相关类型

### Requirement: server route contract tests SHALL avoid direct `SessionState` manipulation

`server` 的 route contract tests MUST 通过 application surface、HTTP 接口或语义化 test harness 搭建场景，SHALL NOT 在测试主体中直接获取 `SessionState` 并手动调用 writer、translator、broadcaster、`prepare_execution()` 或等价 runtime internals。

#### Scenario: route tests 通过语义化 helper 构建已完成 turn
- **WHEN** route contract test 需要一个已完成的 root turn
- **THEN** 它 SHALL 通过语义化 helper 或 application surface 构建该场景
- **AND** 测试主体 SHALL NOT 直接写入 `SessionState.writer`

#### Scenario: busy-session 场景不再直接操作 runtime 状态机
- **WHEN** route contract test 需要一个"当前 session 正在运行"的场景
- **THEN** 它 SHALL 通过 test harness 暴露的语义化 helper 构建该状态
- **AND** 测试主体 SHALL NOT 直接调用 `get_session_state().prepare_execution(...)`

#### Scenario: conversation route-local tests 不再直接构造 runtime replay facts
- **WHEN** 检查 `crates/server/src/http/routes/conversation.rs` 内的 route-local tests
- **THEN** 它们 SHALL 通过 application-owned stream facts 或语义化 fixture 构造测试场景
- **AND** SHALL NOT 直接构造 runtime `ConversationStreamReplayFacts` 或直接持有 runtime projector
