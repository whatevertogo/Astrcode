## MODIFIED Requirements

### Requirement: `session-runtime` 内部继续按单 session 职责分块

`session-runtime` 内部 SHALL 至少按以下职责分块组织，而不是把所有执行细节平铺在 crate 根：

- `state`
- `catalog`
- `actor`
- `turn`
- `context`
- `context_window`
- request assembly 子域（如 `turn/request` 或等价模块）
- `factory`
- `query`

其中子域职责 MUST 满足以下约束：

- `context` 只负责上下文来源、继承与解析结果
- `context_window` 只负责预算、裁剪、压缩与窗口化消息序列
- request assembly 不再长期归属 `context_window`
- `actor` 只负责推进与持有单 session live truth
- `observe` 只负责推送/订阅语义与过滤范围
- `query` 只负责拉取、快照与投影
- `factory` 只负责构造执行输入或执行对象

#### Scenario: 单 session 真相与执行结构清晰

- **WHEN** 检查 `session-runtime/src`
- **THEN** 可以沿着 `state -> actor -> turn -> query` 的结构理解单 session 行为
- **AND** 不需要回到 `application` 中寻找会话真相

#### Scenario: request assembly 不再挂在 context_window 名下

- **WHEN** 检查 `context_window` 子域
- **THEN** 其中只保留预算、裁剪、压缩与窗口化逻辑
- **AND** 最终 request assembly 位于更中性的 request 子域

#### Scenario: query 按读取语义拆分子模块

- **WHEN** 检查 `query` 子域
- **THEN** 其实现至少按 `history`、`agent`、`mailbox`、`turn` 四类读取场景拆分
- **AND** crate 根只保留统一入口与类型导出
