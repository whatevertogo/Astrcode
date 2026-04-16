## MODIFIED Requirements

### Requirement: `actor`、`observe`、`query` 必须按推进、订阅、拉取三类语义分离

`session-runtime` SHALL 固定以下语义边界：

- `actor` 只负责推进与持有单 session live truth
- `observe` 只负责推送/订阅语义、scope/filter、replay/live receiver 与状态源整合
- `query` 只负责拉取、快照与投影

`query` MAY 读取 durable event 与 projected state，但 MUST NOT 负责推进、副作用或长时间持有运行态协调逻辑。conversation/tool display 的 authoritative read model MUST 归属于 `query` 子域；它可以聚合工具调用、流式输出、终态字段与 child 关联等单 session 读取语义，但 MUST NOT 把 HTTP/SSE framing、客户端补丁策略或 surface 样式逻辑带入 `session-runtime`。

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
