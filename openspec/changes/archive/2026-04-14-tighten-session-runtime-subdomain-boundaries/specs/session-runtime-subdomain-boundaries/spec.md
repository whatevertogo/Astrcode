## ADDED Requirements

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

#### Scenario: actor 不再承载观察视图拼装

- **WHEN** 检查 `actor` 子域实现
- **THEN** 其中只包含 session 推进、actor 生命周期与 live truth 管理
- **AND** 不包含 observe 快照投影或外部订阅协议映射

#### Scenario: query 只返回读取结果

- **WHEN** `application` 或 `server` 通过 `SessionRuntime` 发起读取
- **THEN** `query` 子域只返回 snapshot、projection 或 query result
- **AND** 不会因为查询路径隐式追加 durable 事件或推进 turn

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
