## Purpose

`host-session` 作为 session durable truth owner，统一负责事件日志、冷恢复、branch/fork、session catalog、query/read model 与对外 host use-case surface。它通过事件日志维护 durable truth，并驱动 `agent-runtime` 执行。

## Requirements

### Requirement: `host-session` SHALL 成为 session durable truth owner

系统 MUST 新建 `host-session` crate，统一负责事件日志、冷恢复、branch/fork、session catalog、query/read model 与对外 host use-case surface。`host-session` SHALL 通过事件日志维护 durable truth，并驱动 `agent-runtime` 执行，而不是让 live runtime 同时承担全部 durable/session 服务职责。

#### Scenario: host-session 持有事件日志与恢复语义
- **WHEN** 某个 session 需要创建、恢复、分叉或回放
- **THEN** `host-session` SHALL 负责事件追加、投影恢复与 read model 产出
- **AND** `agent-runtime` SHALL 只负责被调用时的 live execution

### Requirement: `host-session` SHALL 替代旧 `application` 的 host 用例入口

`host-session` MUST 暴露稳定的 host-side use-case surface，承接旧 `application` 中与 session、conversation、observe、branch/fork、workflow 驱动相关的正式入口。系统 SHALL 删除 `application` crate，而不是保留一个薄兼容层。

#### Scenario: server 通过 host-session 获取业务用例
- **WHEN** `server` 需要处理 session、conversation、observe、fork、turn 提交等请求
- **THEN** 它 SHALL 通过 `host-session` 暴露的正式 surface 完成
- **AND** SHALL NOT 继续依赖 `application::App`

### Requirement: `host-session` SHALL 通过事件日志驱动 read model

`host-session` MUST 让 conversation snapshot、terminal facts、task facts、branch lineage、mode state 与 turn terminal 等读模型全部来源于事件日志与投影，而不是隐式内存影子状态。

#### Scenario: 服务重启后读模型仍可恢复
- **WHEN** 服务重启后重新读取某个 session 的 conversation snapshot 或 task facts
- **THEN** `host-session` SHALL 通过事件日志恢复读模型
- **AND** SHALL NOT 依赖旧 `application` 或 process-local shadow state

### Requirement: `host-session` SHALL 以新 crate 边界替代旧实现

本次重构完成后，旧 `session-runtime` 中与 event log、query/read model、session catalog、branch/fork 相关的实现 SHALL 迁入 `host-session`，不保留长期兼容 owner。

#### Scenario: 旧 owner 被明确迁出
- **WHEN** 审查最终 crate 职责边界
- **THEN** durable session 服务与 read model SHALL 归属 `host-session`
- **AND** SHALL NOT 继续以"历史原因"留在旧 monolith runtime 中

### Requirement: `host-session` SHALL 维持一 session 即一 agent 的协作真相

系统 MUST 把多 agent 协作建模为"父 session 驱动 child session"，而不是在同一 session 内维护多个可变 agent 身位。`host-session` SHALL 持有 `SubRunHandle`、父子 lineage、`InputQueueProjection`、结果投递与取消传播的 durable truth。

#### Scenario: 发起子 agent 时创建 child session 与 durable linkage
- **WHEN** 父 turn 需要启动一个子 agent
- **THEN** `host-session` SHALL 创建新的 child session 并记录父 turn 到 child session 的 durable 关联
- **AND** SHALL 使用 `SubRunHandle` 与输入队列读模型表达协作状态

#### Scenario: 父 turn 取消时由 host-session 传播到子运行
- **WHEN** 某个父 turn 被取消或中断，且其下存在进行中的 sub-run
- **THEN** `host-session` SHALL 记录取消语义、更新协作状态并向对应 child runtime 传播取消
- **AND** `agent-runtime` SHALL NOT 自己维护另一套 parent/child durable truth
