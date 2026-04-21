## MODIFIED Requirements

### Requirement: 后台调用契约

`SessionRuntime` SHALL 提供 `fork_session(source_session_id, fork_point) -> Result<ForkResult>` 方法。`fork_point` 为 runtime 内部枚举 `StorageSeq(u64) | TurnEnd(String) | Latest`。返回 `ForkResult { new_session_id, fork_point_storage_seq, events_copied }`。不触发任何 turn 执行。

`application` SHALL 提供 `fork_session(session_id, selector) -> Result<SessionMeta>` use case，其中 `selector` MUST 为 application-owned fork selector，而不是 runtime `ForkPoint`。`AppSessionPort` 的实现 SHALL 在 port 边界内部把该 selector 映射为 runtime `ForkPoint`。

#### Scenario: 后台通过 SessionRuntime fork

- **WHEN** 后台流程调用 `SessionRuntime::fork_session`
- **THEN** 返回 `ForkResult` 包含新 session ID、fork 点 storage_seq 和复制的事件数量，不触发 turn 执行

#### Scenario: server 通过 application-owned selector 发起 fork

- **WHEN** `server` 需要从 HTTP 请求触发 session fork
- **THEN** 它 SHALL 通过 `application` 定义的 fork selector 调用 `App::fork_session`
- **AND** SHALL NOT 直接构造 runtime `ForkPoint`

#### Scenario: runtime fork enum 不再穿透到 application 边界

- **WHEN** 检查 `server -> application` 的 fork 调用合同
- **THEN** 对外暴露的类型 SHALL 是 application-owned selector
- **AND** runtime `ForkPoint` SHALL 只留在 application port 实现与 session-runtime 内部

#### Scenario: server 只收到 fork 后的 SessionMeta

- **WHEN** `server` 通过 `application` 发起 fork
- **THEN** `App::fork_session()` SHALL 返回 `SessionMeta`
- **AND** runtime `ForkResult` SHALL 只留在 application / port 内部
