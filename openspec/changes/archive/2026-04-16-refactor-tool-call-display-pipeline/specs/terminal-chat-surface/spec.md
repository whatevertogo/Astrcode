## MODIFIED Requirements

### Requirement: 终端客户端 SHALL 合理呈现 thinking、tool streaming 与子智能体活动

终端客户端 MUST 在有限终端空间内稳定展示 thinking、tool call、stdout / stderr streaming 与 child agent / subagent 活动；主 transcript 与 child 细节视图必须共享同一服务端事实，但终端布局不得要求用户阅读原始低层事件流。工具展示 MUST 直接消费服务端 authoritative tool block，而不是依赖相邻消息分组、低层 stream block 顺序或本地 metadata 猜测来恢复渲染语义。

#### Scenario: 渲染 thinking 片段

- **WHEN** 服务端为当前 assistant 回复持续产出 thinking 内容
- **THEN** 终端客户端 SHALL 将其显示为可识别的 thinking 区块
- **AND** 用户 MUST 能区分 thinking 与最终 assistant 内容

#### Scenario: 渲染工具流式输出

- **WHEN** 某个 tool call 在执行期间持续产出 stdout 或 stderr
- **THEN** 终端客户端 SHALL 增量更新对应 tool block
- **AND** MUST 保留该输出与所属 tool call 的关联关系

#### Scenario: 并发工具交错输出仍保持稳定归属

- **WHEN** 多个 tool call 并发执行且 stdout/stderr 在时间上交错到达
- **THEN** 终端客户端 SHALL 仍能把每段输出稳定渲染到各自的 tool block
- **AND** MUST NOT 因 transcript 相邻顺序变化而串流或错位

#### Scenario: 工具失败与截断信息可直接展示

- **WHEN** 某个 tool call 失败、被截断或在完成后补充 error / duration / truncated 信息
- **THEN** 终端客户端 SHALL 直接显示这些终态字段
- **AND** MUST NOT 通过解析 summary 文本或额外本地推断来补全状态

#### Scenario: 聚焦子智能体视图

- **WHEN** 当前 session 存在运行中或已完成的 child agent / subagent，且用户切换到 child pane 或 focus view
- **THEN** 终端客户端 SHALL 显示该 child 的状态、最近输出与与父会话的关系摘要
- **AND** MUST NOT 把所有 child transcript 无差别平铺到主 transcript 中

#### Scenario: 晚到的子会话关联信息更新同一 tool block

- **WHEN** 某个 tool call 先进入 running / complete 状态，随后才收到与 child session 或 sub-run 关联的补充信息
- **THEN** 终端客户端 SHALL 在原有 tool block 上更新该关联信息
- **AND** MUST NOT 新建重复 tool block 或要求用户手动刷新才能看到关联结果
