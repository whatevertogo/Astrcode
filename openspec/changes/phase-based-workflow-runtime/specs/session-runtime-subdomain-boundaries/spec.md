## ADDED Requirements

### Requirement: `state` 子域 SHALL 只持有 grouped runtime state 与 projection reducers

`session-runtime/state` 子域 MUST 只负责单 session 的 grouped runtime state、projection reducer、durable cache 与相关 typed getter/setter。它 SHALL NOT 承担 workflow 编排、phase 业务语义解释或上层 use-case 判断。

#### Scenario: grouped runtime state 替代散落字段不变量

- **WHEN** `state` 子域维护 active turn、cancel、lease、compacting 与投影缓存
- **THEN** 这些状态 SHALL 以 grouped runtime state 或 projection reducer 的形式暴露
- **AND** SHALL NOT 继续依赖多个互相约束但彼此独立的散落字段维持隐式不变量

#### Scenario: state 子域不解释 workflow business signal

- **WHEN** 上层 workflow 需要解释 approval、replan 或 phase bridge 信号
- **THEN** `state` 子域 SHALL 只提供必要的 authoritative facts
- **AND** SHALL NOT 在该子域内部持有 workflow-specific 分支逻辑

### Requirement: `turn` 子域 SHALL 通过显式 transition API 推进 runtime lifecycle

`session-runtime/turn` 子域推进一次 turn 时 MUST 调用显式的 runtime lifecycle transition API，而不是在多个入口直接写底层状态字段。`submit`、`finalize`、`interrupt` 与 deferred compact 相关路径 SHALL 共享同一组 transition 语义。

#### Scenario: submit 与 finalize 共享统一 transition 入口

- **WHEN** turn 从待执行进入运行中，或从运行中进入终止状态
- **THEN** `submit` 与 `finalize` 路径 SHALL 通过同一组 transition API 更新 runtime lifecycle
- **AND** SHALL NOT 分别直接修改 `active_turn_id`、`lease`、`cancel` 或等价字段

#### Scenario: interrupt 路径复用同一 lifecycle 模型

- **WHEN** 当前 turn 被中断
- **THEN** `interrupt` 路径 SHALL 使用同一 runtime lifecycle 模型把 turn 标记为中断并清理控制状态
- **AND** SHALL NOT 通过单独的旁路状态重置逻辑绕过统一 transition 约束

### Requirement: `session-runtime` SHALL NOT 向 `application` 暴露低层 execution helper

`session-runtime` MUST NOT 直接 re-export `append_and_broadcast`、`prepare_session_execution`、`complete_session_execution` 等低层 helper。`application` SHALL 只通过 `session-runtime` 暴露的稳定 service/facade 接口（如 `SessionRuntime` 的公开方法、`TurnCoordinator`、typed query 方法）消费 runtime 能力，SHALL NOT 直接接触 execution lease、`EventTranslator`、`Phase` lock 或 event append helper。

#### Scenario: application 不直接调用 runtime 低层 helper

- **WHEN** `application` 需要追加事件、切换 mode 或查询 session 状态
- **THEN** 它 SHALL 通过 `SessionRuntime` 的公开方法或 `TurnCoordinator` 生命周期方法完成
- **AND** SHALL NOT 直接调用 `append_and_broadcast`、`prepare_session_execution`、`complete_session_execution` 或直接操作 `SessionState` 内部字段

#### Scenario: session-runtime 收敛公开 API 面

- **WHEN** `session-runtime` 完成 `TurnCoordinator` 和 `ProjectionRegistry` 重构后
- **THEN** 它 SHALL 移除对 `append_and_broadcast`、`prepare_session_execution`、`complete_session_execution` 的 re-export
- **AND** SHALL 只暴露 typed service 方法（如 `submit_prompt`、`switch_mode`、`observe`、query 方法）
- **AND** `application` 侧的测试 SHALL 通过相同的公开 API 面验证行为，不使用低层 helper
