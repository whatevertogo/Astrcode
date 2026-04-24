## ADDED Requirements

### Requirement: hooks 平台 SHALL 成为统一扩展总线

系统 MUST 提供统一的 hooks 平台，作为 builtin 与 external 扩展共享的生命周期总线。该平台 SHALL 至少覆盖 `input`、`context`、`before_agent_start`、`before_provider_request`、`tool_call`、`tool_result`、`turn_start`、`turn_end`、`session_before_compact`、`resources_discover`、`model_select` 这些事件点。

#### Scenario: 同一 turn 触发完整 hook 生命周期
- **WHEN** 一次 turn 从用户输入开始直到完成
- **THEN** 系统 SHALL 在相应时点触发上述 hook 事件
- **AND** builtin 与 external handlers SHALL 通过同一平台接收这些事件

### Requirement: hooks 平台 SHALL 提供明确的分发语义

hooks 平台 MUST 按事件类型提供确定性的分发语义，至少支持顺序执行、可取消、可拦截、可修改、管道式与短路式分发。系统 SHALL NOT 对相同事件类型同时混用未定义的 effect 合并顺序。

#### Scenario: tool_call hook 可拦截
- **WHEN** 某个 `tool_call` hook 返回阻断 effect
- **THEN** 当前工具调用 SHALL 被阻止执行
- **AND** 系统 SHALL 产生可观测的阻断结果

#### Scenario: context hook 以管道方式修改输入
- **WHEN** 多个 `context` hooks 同时命中
- **THEN** 后一个 handler SHALL 接收前一个 handler 的输出
- **AND** 合并顺序 SHALL 保持稳定

### Requirement: governance prompt hooks SHALL 走统一 hooks 平台

所有 turn-scoped prompt augment 行为 MUST 作为 hooks 平台中的标准 effect 处理，并继续通过 `PromptDeclaration` / `PromptGovernanceContext` 进入既有 prompt 组装链路。系统 SHALL NOT 再维护平行的 governance prompt 特判系统。

#### Scenario: hook 产出的 prompt augment 进入既有 prompt 管线
- **WHEN** 某个 hook 需要为当前 turn 注入额外 prompt declarations
- **THEN** 系统 SHALL 将其转换为 `PromptDeclaration`
- **AND** SHALL 沿既有 prompt 组装链路进入最终请求

### Requirement: hooks 平台 SHALL 只允许受约束的 effect

hooks 平台 MUST 将 handlers 产出的 effect 限制在可验证的受约束集合内，例如阻断、取消当前 turn、上下文补充、prompt augment、结果修饰、资源发现或模型选择建议。hook SHALL NOT 直接突变 session durable truth 或绕过 host/runtime owner 写入内部状态。

#### Scenario: hook 不得直接写 durable truth
- **WHEN** 某个 hook 尝试直接修改 session durable state
- **THEN** 系统 SHALL 拒绝该 effect
- **AND** durable truth 仍 SHALL 只由正式 owner 写入

#### Scenario: `cancel_turn` 只能短路当前 turn
- **WHEN** 某个 hook 返回 `cancel_turn` effect
- **THEN** 当前 turn SHALL 被有界地终止，并返回可解释的终止结果
- **AND** hook SHALL NOT 借此直接写入或篡改 session durable truth

### Requirement: hooks 平台 SHALL 为每个 hook 事件定义明确的 owner、触发点与分发模式

hooks 平台 MUST 为每个正式事件定义唯一 owner、触发时机与分发模式，避免同一事件同时被多个层随意触发。第一阶段与 `pi-mono` 对齐的正式事件目录如下：

| 事件 | owner | 触发点 | 分发模式 | 当前用途 |
| --- | --- | --- | --- | --- |
| `input` | `host-session` | 接收用户输入后、进入 turn 前 | 短路 / 转换 | 输入拦截、预处理、重写 |
| `context` | `agent-runtime` | 组装 provider 上下文前 | 管道 | 上下文裁剪、补充、注入 |
| `before_agent_start` | `agent-runtime` | system prompt 组装完成后、loop 启动前 | 管道 | prompt augment、治理 overlay |
| `before_provider_request` | `agent-runtime` | 发起 provider 请求前 | 管道 | 路由、鉴权、payload 修饰 |
| `tool_call` | `agent-runtime` | 工具执行前 | 拦截 | 参数修补、策略阻断 |
| `tool_result` | `agent-runtime` | 工具执行后 | 修改 | 结果修饰、错误重分类 |
| `turn_start` | `agent-runtime` | turn 开始时 | 顺序 | 观测、初始化 |
| `turn_end` | `agent-runtime` | turn 结束时 | 顺序 | 清理、汇总、通知 |
| `session_before_compact` | `host-session` | 压缩执行前 | 可取消 / 可修改 | 阻止压缩、定制压缩输入 |
| `resources_discover` | `plugin-host` | 构建资源目录前 | 聚合 | 贡献 skills/prompts/themes 等资源路径 |
| `model_select` | `host-session` | 模型切换或恢复时 | 可取消 / 可修改 | 模型准入、重定向、降级 |

#### Scenario: 每个事件只有一个正式 owner
- **WHEN** 系统实现某个 hooks 事件
- **THEN** 该事件 SHALL 由上表指定的 owner 触发
- **AND** 其他层 SHALL NOT 以同名事件重复触发第二次

#### Scenario: event 目录可直接映射到实现位置
- **WHEN** 实现者查找某个 hook 的触发逻辑
- **THEN** 能从事件名直接定位到 `agent-runtime`、`host-session` 或 `plugin-host` 中的单一 owner
- **AND** SHALL NOT 需要在多个 crate 中猜测哪个才是真实触发点

### Requirement: 第一阶段正式 hook 事件 SHALL 具备明确使用点

第一阶段正式 hooks 事件 MUST 具备明确的使用点，而不是只保留事件名。每个事件至少满足以下语义：

- `input`：允许 host 在 turn 创建前阻断、转换或完全处理输入。
- `context`：允许 runtime 在 provider 调用前对消息上下文做链式变换。
- `before_agent_start`：允许把 workflow / governance / mode overlay 转成 `PromptDeclaration`。
- `before_provider_request`：允许 provider payload 被代理、路由或加签。
- `tool_call`：允许阻断工具、修补参数或施加更严格策略。
- `tool_result`：允许工具结果被修饰、截断、重分类。
- `turn_start` / `turn_end`：允许做 turn 粒度观测、初始化和收尾。
- `session_before_compact`：允许压缩被取消、定制或由外部摘要取代。
- `resources_discover`：允许 plugin-host 聚合 skills / prompts / themes / commands 等资源入口。
- `model_select`：允许模型选择被策略校验、重定向或拒绝。

#### Scenario: `input` hook 在创建 turn 前工作
- **WHEN** 用户输入到达系统但尚未创建 turn
- **THEN** `input` hook SHALL 有机会返回“继续”“转换后继续”或“已处理”
- **AND** host SHALL 根据结果决定是否创建新 turn

#### Scenario: `tool_call` 与 `tool_result` 分别负责前置拦截和后置修饰
- **WHEN** 某次工具调用发生
- **THEN** `tool_call` SHALL 在工具执行前运行
- **AND** `tool_result` SHALL 在工具执行后运行
- **AND** 这两个事件 SHALL NOT 互相替代

#### Scenario: `resources_discover` 聚合完整资源面
- **WHEN** `plugin-host` 组装当前可用资源目录
- **THEN** `resources_discover` SHALL 允许贡献 `skills`、`prompts`、`themes`、`commands` 等资源入口
- **AND** SHALL NOT 只局限于 tool 或 capability 发现

### Requirement: 第二阶段预留 hook 事件 SHALL 先保留正式事件名与未来使用点

为与 `pi-mono` 对齐并避免未来再次引入私有回调，hooks 平台 MUST 为下一阶段预留正式事件名、owner 与未来使用点。以下事件在本 change 中不要求全部实现，但 SHALL 作为正式 hook catalog 的保留项存在：

| 预留事件 | owner | 未来使用点 |
| --- | --- | --- |
| `session_start` | `host-session` | session 初始化、默认资源装载、首次治理注入 |
| `session_before_switch` | `host-session` | session 切换前阻断或清理 |
| `session_before_fork` | `host-session` | fork 前校验、摘要策略 |
| `session_compact` | `host-session` | 压缩完成后的观测与补充记录 |
| `session_shutdown` | `host-session` | reload、退出、session replacement 前清理 |
| `session_before_tree` | `host-session` | branch/tree 导航前拦截、摘要覆盖 |
| `session_tree` | `host-session` | branch/tree 导航后的观测与同步 |
| `session_before_spawn_child` | `host-session` | child session 创建前校验、路由、命名、limits 注入 |
| `subrun_start` | `host-session` | sub-run durable linkage 建立后的观测 |
| `subrun_end` | `host-session` | child turn terminal 后的收尾、通知、摘要 |
| `subrun_result_delivery` | `host-session` | 结果回传父 session 前的过滤、摘要、路由 |
| `after_provider_response` | `agent-runtime` | provider metadata、headers、retry hint 观测 |
| `agent_start` | `agent-runtime` | agent loop 生命周期观测 |
| `agent_end` | `agent-runtime` | agent loop 结束汇总 |
| `message_start` | `agent-runtime` | 消息级生命周期通知 |
| `message_update` | `agent-runtime` | 流式 token/message 更新观测 |
| `message_end` | `agent-runtime` | 消息结束汇总 |
| `tool_execution_start` | `agent-runtime` | 工具执行开始观测 |
| `tool_execution_update` | `agent-runtime` | 工具流式更新观测 |
| `tool_execution_end` | `agent-runtime` | 工具执行结束观测 |
| `user_bash` | `host-session` 或终端 host | 终端专属 shell shortcut 与执行代理 |

#### Scenario: 未来扩展继续沿正式 catalog 增长
- **WHEN** 系统后续需要扩展 session tree、message streaming 或 tool execution 粒度的 hooks
- **THEN** 应直接实现上述预留事件
- **AND** SHALL NOT 再新增一套平行的私有 callback surface

### Requirement: hooks 平台 SHALL 为 streaming 与 observability 事件保留非真相语义

`after_provider_response`、`message_start/update/end`、`tool_execution_start/update/end`、`agent_start/end` 这类 streaming / observability 事件 MUST 明确为“观测或 UI 协调事件”，它们 SHALL NOT 成为 session durable truth 的写入入口。

#### Scenario: message/tool execution hooks 只做观测与附加行为
- **WHEN** 某个 streaming 或 tool execution 事件触发
- **THEN** handler MAY 记录指标、发送 UI 更新或附加诊断
- **AND** SHALL NOT 直接重写 durable transcript 真相
