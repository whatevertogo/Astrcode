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

turn terminal 等待、running turn watcher 与等价的运行态等待逻辑 SHALL NOT 归属于 `query`；它们 MUST 归属于 `turn` 子域。

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

#### Scenario: query 不再拥有 turn terminal 等待循环

- **WHEN** 检查 `query/service.rs` 或等价 query façade
- **THEN** 其中不再包含 `wait_for_turn_terminal_snapshot()` 这类基于 broadcaster 的等待循环
- **AND** turn terminal 等待 SHALL 归属 `turn/watcher.rs` 或等价的 turn-owned 模块

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

`session-runtime/state` 子域 MUST 只负责 durable projection state、projection reducer、durable cache、writer/broadcast 基础设施与相关 typed getter/setter。它 SHALL NOT 承担 turn runtime lifecycle control、workflow 编排、phase 业务语义解释或上层 use-case 判断。

`TurnRuntimeState`、`CompactRuntimeState`、`ActiveTurnState`、`ForcedTurnCompletion`、`PendingManualCompactRequest` 等运行时控制类型 MUST 归属于 `turn` 子域，而不是 `state`。

#### Scenario: grouped runtime state 替代散落字段不变量

- **WHEN** `state` 子域维护 active turn、cancel、lease、compacting 与投影缓存
- **THEN** 这些状态 SHALL 以 grouped runtime state 或 projection reducer 的形式暴露
- **AND** SHALL NOT 继续依赖多个互相约束但彼此独立的散落字段维持隐式不变量

#### Scenario: state 子域不解释 workflow business signal

- **WHEN** 上层 workflow 需要解释 approval、replan 或 phase bridge 信号
- **THEN** `state` 子域 SHALL 只提供必要的 authoritative facts
- **AND** SHALL NOT 在该子域内部持有 workflow-specific 分支逻辑

#### Scenario: state 子域只保留 durable/projection 真相

- **WHEN** `state` 子域维护 phase、mode、turn projection、child sessions、tasks 与 input queue
- **THEN** 这些状态 SHALL 继续以 projection reducer 或 durable cache 的形式存在
- **AND** `state` SHALL NOT 再持有 active turn、cancel token、turn lease 或 compact runtime control

#### Scenario: SessionState 不再提供 turn runtime proxy

- **WHEN** 检查 `SessionState` 的公开方法
- **THEN** 不再存在 `prepare_execution()`、`complete_execution_state()`、`interrupt_execution_if_running()`、`cancel_active_turn()`、`is_running()`、`active_turn_id_snapshot()`、`manual_compact_pending()`、`compacting()`、`set_compacting()`、`request_manual_compact()` 等 turn runtime proxy
- **AND** turn 路径 SHALL 直接通过 turn-owned runtime handle 推进控制状态

#### Scenario: 外部 crate 不再通过 SessionState proxy 搭建测试场景

- **WHEN** `application` 或 `server` 的测试需要构造 running turn、completed turn 或 deferred compact 场景
- **THEN** 它们 SHALL 通过 `SessionRuntime` 稳定 façade、调用方本地 test support 或语义化 helper 搭建
- **AND** SHALL NOT 继续依赖 `SessionState` runtime proxy

### Requirement: `turn` 子域 SHALL 通过显式 transition API 推进 runtime lifecycle

`session-runtime/turn` 子域推进一次 turn 时 MUST 调用显式的 runtime lifecycle transition API，而不是在多个入口直接写底层状态字段。`submit`、`finalize`、`interrupt`、deferred compact 与 turn terminal watcher 相关路径 SHALL 共享同一组 turn-owned runtime lifecycle 语义。

#### Scenario: submit 与 finalize 共享统一 transition 入口

- **WHEN** turn 从待执行进入运行中，或从运行中进入终止状态
- **THEN** `submit` 与 `finalize` 路径 SHALL 通过同一组 transition API 更新 runtime lifecycle
- **AND** SHALL NOT 分别直接修改 `active_turn_id`、`lease`、`cancel` 或等价字段

#### Scenario: interrupt 路径复用同一 lifecycle 模型

- **WHEN** 当前 turn 被中断
- **THEN** `interrupt` 路径 SHALL 使用同一 runtime lifecycle 模型把 turn 标记为中断并清理控制状态
- **AND** SHALL NOT 通过单独的旁路状态重置逻辑绕过统一 transition 约束

#### Scenario: watcher 归 turn 子域所有

- **WHEN** 上层需要等待某个 turn 到达可判定终态
- **THEN** 系统 SHALL 通过 `turn` 子域提供的 watcher 能力完成
- **AND** watcher MAY 订阅 broadcaster 并在 lagged / closed 时回放恢复
- **AND** 这类等待语义 SHALL NOT 继续放在 `query` 子域

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

### Requirement: `session-runtime` SHALL 分离 external snapshots、durable event truth 与 runtime control state

`session-runtime` MUST 同时承认并分离三类语义层：

- external snapshots / result contracts：提供给 `application` / `server` 的纯数据结果
- durable event truth：唯一的 append-only 会话事实来源
- runtime control state：仅在运行期间存在的取消、并发、lease 与执行控制状态

其中只有前两类可以作为外层稳定合同；runtime control state SHALL 保持在 runtime 内部，SHALL NOT 通过编排合同直接暴露。

#### Scenario: application 和 server 只消费纯数据合同
- **WHEN** `application` 或 `server` 读取 session facts、turn outcome、observe 摘要或 terminal 相关状态
- **THEN** 它们 SHALL 获得纯数据 snapshot / DTO / result
- **AND** SHALL NOT 直接持有 `CancelToken`、锁对象、原子状态或其他 runtime control primitive

#### Scenario: runtime control state 不成为第二套 durable truth
- **WHEN** turn 运行期间使用 active turn、running、generation、lease 或 cancel 等控制状态
- **THEN** 这些状态 SHALL 只作为进程内运行时控制信息存在
- **AND** durable 可恢复事实 SHALL 继续通过事件流和投影表达

### Requirement: 跨 runtime 边界的扩展点 SHALL 只交换纯数据 context / result

凡是跨出 `session-runtime` 边界的扩展点，例如上层消费的 session contracts、订阅载荷、hook context/outcome、policy context/verdict、capability/tool 注册描述等，SHALL 只交换纯数据 context / result。它们 MAY 承载可序列化 snapshot、事件、声明和决策，但 SHALL NOT 直接暴露 runtime control primitives。

#### Scenario: 外部扩展点不暴露 runtime 内脏
- **WHEN** 某个能力、hook、policy 或上层 session 合同跨出 runtime 边界
- **THEN** 它 SHALL 只包含可序列化、可比较的纯数据字段
- **AND** SHALL NOT 直接暴露 `CancelToken`、锁对象、原子状态、active turn handle 或等价 runtime control primitive

#### Scenario: runtime-local 组合细节不被误判为外部合同
- **WHEN** server/application 组合期内部需要持有 receiver、handle 或其他本地运行时对象
- **THEN** 这些对象 MAY 作为组合根内部实现细节存在
- **AND** 只要它们没有作为跨 runtime 边界的正式输入输出暴露，就不视为违反纯数据合同约束

### Requirement: `query` 子域 SHALL 成为编排侧读取 helper 的唯一所有者

凡是面向编排消费者的单 session 读取 helper，例如 turn terminal、turn outcome、observe 摘要、recoverable delivery 聚合等，`session-runtime` SHALL 以 `query` 子域为唯一长期所有者。`turn`、`command` 与外层 crate MAY 触发这些读取，但 SHALL NOT 长期保留同类投影与聚合实现。与这些读取 helper 对应的纯投影算法 MAY 位于共享 reducer / projector 模块中，但 `query` 继续拥有面向外部的读取 API。

#### Scenario: query/service 只编排读取流程，不复制投影算法
- **WHEN** `query/service` 提供 turn terminal wait、turn outcome projection 或 recoverable delivery 读取能力
- **THEN** 它 SHALL 调用 `query` 子域内部的 canonical helper
- **AND** SHALL NOT 在 service 层继续复制事件扫描、终态判断或摘要聚合逻辑

#### Scenario: turn 子域复用 query canonical helper
- **WHEN** `turn` finalize 或等价执行路径需要读取某类已存在的 query 事实
- **THEN** 它 SHALL 复用 `query` 子域的 canonical helper 或已缓存事实
- **AND** SHALL NOT 因为身处执行路径就重新维护一套同语义的聚合代码

### Requirement: transcript / session replay 的只读 API SHALL 属于 `query` 子域

`session_transcript_snapshot`、`session_replay` 和等价的 transcript/session replay 只读能力 MUST 归属于 `query` 子域，SHALL NOT 继续长期放在 `turn/` 名下。

#### Scenario: replay 读取 API 不再留在 turn 子域
- **WHEN** 检查 transcript/session replay 的实现归属
- **THEN** 它们 SHALL 位于 `query` 子域
- **AND** `turn/` SHALL 只保留执行、提交、终结与运行时控制相关逻辑

### Requirement: `turn` 子域 SHALL NOT 反向依赖 `query` 组装执行输入

`turn` 子域负责执行生命周期和请求推进，`query` 子域负责读取投影结果。`turn` 在准备执行输入时 MAY 读取 `SessionState` 的快照或专门的 neutral helper，但 SHALL NOT 直接依赖 `query::*` 组装当前 turn 消息、终态或等价读取语义。

#### Scenario: submit 不再 import query helper
- **WHEN** `turn/submit` 组装当前 turn 的消息输入
- **THEN** 它 SHALL 通过 `SessionState` 的直接快照 API 或等价 neutral helper 获取所需消息
- **AND** SHALL NOT 直接 import `query::current_turn_messages` 或等价 query helper

#### Scenario: interrupt 不再调用 submit 内部持久化 helper
- **WHEN** interrupt 路径需要处理 deferred compact 或等价 finalize 后续动作
- **THEN** 它 SHALL 调用独立的 finalize / compact helper
- **AND** SHALL NOT 通过 `submit` 内部私有语义形成子域双向耦合

#### Scenario: wait-for-terminal 语义暂不在本次迁移
- **WHEN** 检查 `wait_for_turn_terminal_snapshot()` 的实现归属
- **THEN** 本次 change MAY 暂时保持其在 `query/service` 中
- **AND** 该等待/观察语义的进一步迁移 SHALL 留给后续独立 change

### Requirement: `ProjectionRegistry` SHALL 退化为薄协调器并委托域 reducer

`ProjectionRegistry` MUST 作为统一入口保留，但其职责 SHALL 收窄为固定顺序的 apply / snapshot 协调；turn、children、tasks、input_queue、recent cache 等域逻辑 SHALL 由独立 reducer/owner 承担，registry 本身 SHALL NOT 长期堆积跨域细节与命令式后门。

#### Scenario: child/task/input/turn 各域拥有独立 reducer
- **WHEN** 系统维护 child nodes、active tasks、input queue 和 turn terminal projections
- **THEN** 每个域 SHALL 拥有独立的 reducer/owner 负责 `apply` / `snapshot` / `rebuild`
- **AND** `ProjectionRegistry` SHALL 只负责按固定顺序委托

#### Scenario: registry 根对象不再持有跨域命令式后门
- **WHEN** 某个投影域需要支持局部更新或兼容迁移
- **THEN** 该更新入口 SHALL 收敛到对应域 reducer 内部
- **AND** `ProjectionRegistry` 根对象 SHALL NOT 继续扩张出新的跨域命令式 mutation helper

### Requirement: input queue 的命令追加路径 SHALL 属于 `command` 子域

`InputQueueEventAppend`、`append_input_queue_event` 与等价的 input queue durable 写路径 MUST 属于 `command` 子域；`state/input_queue` SHALL 只保留 input queue 投影、索引更新和读取相关逻辑。

#### Scenario: state/input_queue 不再承载写命令
- **WHEN** 检查 `state/input_queue` 子域
- **THEN** 其中 SHALL 只保留 input queue projection / reducer / 读取辅助逻辑
- **AND** durable append 命令 SHALL 位于 `command` 子域

### Requirement: `session-runtime` SHALL 通过稳定 facade 阻断 `application` 对内部 helper 的直接依赖

`session-runtime` 必须通过稳定 façade 阻断 `application` 对内部 helper 的直接依赖。`application` SHALL 只通过 `SessionRuntime` 公开方法或 `AppSessionPort` / `AgentSessionPort` 对应合同读取或推进 session 事实，SHALL NOT 直接调用路径规范化函数、低层 execution helper 或内部投影器。

#### Scenario: application 不直接调用 runtime helper
- **WHEN** `application` 需要标准化 `session_id`、等待 turn 终态、观察 child session 或恢复 parent delivery
- **THEN** 它 SHALL 通过 `session-runtime` 的稳定 façade 或 port trait 完成
- **AND** SHALL NOT 直接依赖 `normalize_session_id`、`append_and_broadcast` 或等价内部 helper

#### Scenario: server 测试与上层调用跟随稳定 façade
- **WHEN** 上层测试或调用方需要构造 session 行为
- **THEN** 它们 SHALL 优先通过稳定 façade 或应用层合同完成验证
- **AND** 本次 change 完成后 SHALL 不再新增绕过 façade 的 helper 级调用
