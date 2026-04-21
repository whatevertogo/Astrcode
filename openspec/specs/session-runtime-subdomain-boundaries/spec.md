## Purpose

定义 `session-runtime` 内部子域的职责边界，确保模块组织清晰且长期可维护。

## Requirements

### Requirement: `context` 与 `context_window` 必须分离来源解析和预算窗口职责

`session-runtime` SHALL 将上下文来源解析与预算窗口管理建模为两个独立子域：

- `context` 只负责上下文来源、继承关系、解析结果与结构化快照
- `context_window` 只负责预算、裁剪、压缩与窗口化消息序列

最终 request assembly MUST NOT 继续长期归属 `context_window`。

#### Scenario: context 产出结构化快照而非最终请求

- **WHEN** 执行流程需要读取本次 turn 的可用上下文
- **THEN** `context` 子域返回结构化解析结果，例如 `ResolvedContextSnapshot`
- **AND** 不直接产出最终执行请求或已组装 prompt

#### Scenario: context_window 只负责预算内窗口化

- **WHEN** 执行流程需要根据 token 预算裁剪消息
- **THEN** `context_window` 负责预算、裁剪、压缩和窗口化消息序列
- **AND** 不承担 request assembly 的最终所有权

---

### Requirement: `actor`、`observe`、`query` 必须按推进、订阅、拉取三类语义分离

`session-runtime` SHALL 固定以下语义边界：

- `actor` 只负责推进与持有单 session live truth
- `observe` 只负责推送/订阅语义、scope/filter、replay/live receiver 与状态源整合
- `query` 只负责拉取、快照与投影

`query` MAY 读取 durable event 与 projected state，但 MUST NOT 负责推进、副作用或长时间持有运行态协调逻辑。

conversation/tool display 的 authoritative read model MUST 归属于 `query` 子域；它可以聚合工具调用、流式输出、终态字段与 child 关联等单 session 读取语义，但 MUST NOT 把 HTTP/SSE framing、客户端补丁策略或 surface 样式逻辑带入 `session-runtime`。

#### Scenario: actor 不再承载观察视图拼装

- **WHEN** 检查 `actor` 子域实现
- **THEN** 其中只包含 session 推进、actor 生命周期与 live truth 管理
- **AND** 不包含 observe 快照投影或外部订阅协议映射

#### Scenario: query 只返回读取结果

- **WHEN** `application` 或 `server` 通过 `SessionRuntime` 发起读取
- **THEN** `query` 子域只返回 snapshot、projection 或 query result
- **AND** 不会因为查询路径隐式追加 durable 事件或推进 turn

#### Scenario: conversation 工具展示聚合落在 query 子域

- **WHEN** 上层需要读取某个 session 的工具展示结构、conversation hydration 或 catch-up 结果
- **THEN** `query` 子域 SHALL 直接返回 authoritative conversation/tool display facts
- **AND** 上层 MUST NOT 重新从原始 transcript record 或 replay/live receiver 组装同类语义

#### Scenario: observe 不承载 UI 级工具聚合语义

- **WHEN** `observe` 暴露 replay/live receiver 或相关订阅结果
- **THEN** 它 SHALL 只表达订阅与恢复语义
- **AND** MUST NOT 成为 tool block、conversation block 或等价 UI 读模型的长期所有者

---

### Requirement: `factory` 只能负责构造执行输入或执行对象

`session-runtime/factory` SHALL 只承担构造类职责，包括执行输入或执行对象的构造。

`factory` MUST NOT 承担：

- 策略决策
- 输入校验
- 状态读写
- 业务权限判断

#### Scenario: factory 保持无状态构造定位

- **WHEN** 检查 `factory` 子域实现
- **THEN** 其职责仅限构造执行输入、lease 或等价执行对象
- **AND** 不直接依赖会话状态读写或业务策略分支

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
