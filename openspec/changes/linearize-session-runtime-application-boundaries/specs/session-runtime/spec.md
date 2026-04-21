## ADDED Requirements

### Requirement: `session-runtime` SHALL 为重复的 turn/query helper 指定单一 canonical owner

`session-runtime` MUST 为 turn 终态投影、assistant summary 提取和 `session_id` 规范化等重复 helper 指定单一 canonical owner。其他子域调用方 SHALL 只复用该实现，SHALL NOT 继续在 `query/service`、`turn/submit`、`application` 或等价位置各自维护一份同类逻辑。

#### Scenario: turn outcome 与 terminal snapshot 复用同一投影逻辑
- **WHEN** 系统需要计算某个 turn 的 terminal snapshot 或 projected outcome
- **THEN** `query/service` 与其他消费方 SHALL 通过 `query/turn` 的 canonical helper 生成结果
- **AND** SHALL NOT 在多个调用点分别扫描事件并各自拼装相同语义

#### Scenario: assistant summary 提取不再多处实现
- **WHEN** finalize 路径或查询路径需要读取某个 turn 的 assistant summary
- **THEN** 系统 SHALL 通过同一份 summary 提取 helper 或 reducer 获取结果
- **AND** SHALL NOT 在 `turn/submit` 与 `query/turn` 中长期保留两套等价实现

#### Scenario: session id 规范化只有一个所有者
- **WHEN** 任意运行时入口需要把外部 `session_id` 输入转换为内部使用形式
- **THEN** 系统 SHALL 通过 `state::paths` 或等价 typed helper 完成规范化
- **AND** `application` 与多个 runtime 调用点 SHALL NOT 继续散落手写等价规范化逻辑

### Requirement: turn terminal projection SHALL 由同一 projector 同时服务增量、回放和重建路径

同一个 turn 的 terminal projection MUST 由一套共享 projector/reducer 逻辑生成。live append、query replay fallback 和 recovery rebuild SHALL 共用该逻辑，SHALL NOT 继续长期维护两套以上对 `TurnDone` / `Error` 的平行匹配分支。

#### Scenario: projection registry 与 query 共享同一 turn projector
- **WHEN** live append 更新某个 turn 的 terminal projection
- **THEN** `ProjectionRegistry` SHALL 通过共享 turn projector/reducer 更新结果
- **AND** query fallback SHALL 复用同一 projector 逻辑

#### Scenario: rebuild 与 live append 产出一致 terminal projection
- **WHEN** 系统分别通过 recovery rebuild 和 live append 处理等价的 turn 事件序列
- **THEN** 它们 SHALL 产出相同的 `TurnProjectionSnapshot`
- **AND** SHALL NOT 因为走不同入口而出现 terminal kind / last error 漂移

### Requirement: post-compact durable events SHALL 由共享 builder 生成

主动 compact、reactive compact 和 manual compact 之后写入的 durable 事件序列 MUST 由共享 builder 生成。该 builder SHALL 统一负责 `compact_applied`、recent user context digest/messages 和 file recovery messages 的构造；各调用方只负责提供 trigger、上下文与 compact result。

#### Scenario: 不同 compact 路径复用同一事件 builder
- **WHEN** proactive、reactive 或 manual compact 成功完成
- **THEN** 系统 SHALL 通过同一共享 builder 生成后续 durable 事件序列
- **AND** SHALL NOT 在三个调用点长期维护三套等价的事件组装逻辑

#### Scenario: compact 事件序列在不同 trigger 下结构保持一致
- **WHEN** 仅 compact trigger 不同，但 compact result 结构等价
- **THEN** 生成的 post-compact durable 事件结构 SHALL 保持一致
- **AND** 不同路径的差异 SHALL 仅来自 trigger 和对应上下文值，而不是事件拼装规则分叉

### Requirement: `session-runtime` crate 根导出面 SHALL 收口到稳定 façade 与稳定事实

`session-runtime` crate 根的公开导出 MUST 只保留稳定 façade、稳定 snapshot/result 和确实面向外层合同的 read-model facts。低层 orchestration helper、路径规范化函数和仅用于 runtime 内部拼装的辅助类型 SHALL NOT 继续作为 crate 根默认导出面。

#### Scenario: orchestration helper 不再从 crate 根外泄
- **WHEN** 外层 crate 依赖 `session-runtime`
- **THEN** 它们 SHALL 通过 `SessionRuntime` 的公开方法或 port blanket impl 消费运行时能力
- **AND** SHALL NOT 依赖 crate 根暴露的低层 helper、执行辅助或路径规范化工具完成编排

#### Scenario: 稳定 read-model facts 仍可继续暴露
- **WHEN** 某个类型已经作为 terminal / conversation 的稳定 authoritative facts 被上层 surface 消费
- **THEN** `session-runtime` MAY 继续公开该类型
- **AND** 本次收口 SHALL 聚焦 orchestration helper 与内部运行时辅助，不把 terminal read-model 的后续隔离强行并入同一阶段
