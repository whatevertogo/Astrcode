## ADDED Requirements

### Requirement: `application` SHALL 通过 app-owned session orchestration contracts 隔离 runtime 内部类型

`application` MUST 为编排场景定义 app-owned session orchestration contracts，并通过这些合同消费 `session-runtime` / `kernel` 提供的事实。用于 turn terminal、turn outcome、observe 摘要、recoverable parent delivery 等编排语义的 port 返回值 SHALL NOT 继续直接暴露 `session-runtime` 或 `kernel` 的内部快照类型。

#### Scenario: AgentSessionPort 不再暴露 runtime/kernel 内部快照
- **WHEN** `AgentSessionPort` 提供 observe、turn outcome、turn terminal 或 recoverable delivery 能力
- **THEN** 其返回类型 SHALL 使用 `application` 定义的 contract DTO
- **AND** SHALL NOT 继续直接暴露 `ProjectedTurnOutcome`、`TurnTerminalSnapshot`、`AgentObserveSnapshot`、`PendingParentDelivery` 或等价内部类型

#### Scenario: blanket impl 负责映射底层事实
- **WHEN** `SessionRuntime` 作为 `AppSessionPort` / `AgentSessionPort` 的实现被注入 `application`
- **THEN** blanket impl SHALL 在 port 层把 runtime/kernel 事实映射为 app-owned contracts
- **AND** `application` 用例本身 SHALL 不感知底层快照结构

#### Scenario: app-owned contracts 保持纯数据
- **WHEN** `application` 定义 session orchestration contracts
- **THEN** 这些 contracts SHALL 只包含纯数据字段与可序列化/可比较的业务结果
- **AND** SHALL NOT 直接承载 `CancelToken`、锁对象、原子状态、channel handle 或其他 runtime control primitive

### Requirement: `application` SHALL NOT 通过 `lib.rs` re-export 继续泄漏仅供编排内部使用的 runtime 类型

`application` crate 根导出面 MUST 只保留稳定业务入口、稳定业务摘要和确有必要的共享 surface。仅供内部编排使用的 runtime 类型 SHALL NOT 继续通过 `application::lib.rs` re-export 暴露给 `server` 或其他上层调用方。

#### Scenario: orchestration-only runtime types 从应用层根导出面移除
- **WHEN** 检查 `application::lib.rs`
- **THEN** 仅用于内部编排的 runtime 类型 SHALL 不再被 re-export
- **AND** 上层调用方 SHALL 通过 `App`、typed summary 或后续专门 surface 消费等价能力

#### Scenario: terminal authoritative facts 暂时保持稳定导出
- **WHEN** 某类 runtime facts 已经被 terminal / conversation surface 作为 authoritative read model 直接消费
- **THEN** `application` MAY 在本阶段继续保留必要导出
- **AND** 本次 change SHALL 聚焦编排合同隔离，不把 terminal read-model 全量迁移并入同一阶段

### Requirement: `application` SHALL 把 session 输入规范化留在 port 实现内部

`application` 用例层 MUST 把外部 session 输入视为原始请求数据；`session_id` 的规范化、typed conversion 与等价 runtime path helper 调用 SHALL 由 `AppSessionPort` / `AgentSessionPort` 的实现内部负责。应用层用例 SHALL NOT 直接调用 `astrcode_session_runtime::normalize_session_id` 或等价 helper。

#### Scenario: use case 只做字段校验，不做 runtime 规范化
- **WHEN** `application` 处理 session 相关请求
- **THEN** 它 MAY 做空值、格式非法等字段级校验
- **AND** SHALL NOT 直接依赖 runtime 的路径或 id 规范化 helper

#### Scenario: runtime 实现内部完成 session id 标准化
- **WHEN** 原始 `session_id` 进入 `AppSessionPort` / `AgentSessionPort` 的具体实现
- **THEN** 实现层 SHALL 在调用 runtime 内部逻辑前完成标准化与 typed conversion
- **AND** 该标准化语义 SHALL 与 `session-runtime` 内部 canonical helper 保持一致
