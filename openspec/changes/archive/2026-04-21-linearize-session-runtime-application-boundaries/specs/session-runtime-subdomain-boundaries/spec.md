## ADDED Requirements

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
