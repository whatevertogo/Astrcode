# terminal-chat-surface Specification

## Purpose
定义正式终端聊天客户端作为一等产品 surface 的用户可见能力与交互边界。

## Requirements

### Requirement: 终端客户端 SHALL 作为依赖现有 `astrcode-server` 的一等聊天入口

正式发布的终端版 Astrcode MUST 作为独立 client surface 连接现有 `astrcode-server`，通过统一 bootstrap、认证交换与会话 API 工作，而不是直接嵌入 `application`、`kernel` 或 `session-runtime` 并形成第二套组合根。

#### Scenario: 附着到已运行的本地 server

- **WHEN** 用户启动终端客户端且本地 `run.json` 指向一个已运行的 `astrcode-server`
- **THEN** 终端客户端 SHALL 通过现有 bootstrap / auth exchange 完成附着
- **AND** MUST NOT 再启动第二个本地 runtime 或维护平行会话真相

#### Scenario: 托管拉起本地 server

- **WHEN** 用户未显式指定 `--server-origin` 且本地没有可附着的 server
- **THEN** 终端客户端 SHALL 能按正式 ready 协议拉起并附着 `astrcode-server`
- **AND** 若拉起或 ready 握手失败，MUST 向用户返回明确错误而不是停留在无响应状态

#### Scenario: 远程连接认证失败

- **WHEN** 用户通过 `--server-origin` 或等价配置连接远程 server，但 token 无效或 exchange 失败
- **THEN** 终端客户端 SHALL 拒绝进入聊天主界面
- **AND** MUST 明确告知认证失败原因与当前连接目标

### Requirement: 终端客户端 SHALL 覆盖完整聊天工作流

终端客户端 MUST 支持完整的聊天产品面，包括新建会话、恢复会话、切换会话、提交 prompt、消费流式回复，以及在会话恢复后继续沿用同一服务端真相，而不是退化成只读观测界面。

#### Scenario: 新建会话并开始流式对话

- **WHEN** 用户在终端中创建新会话并提交第一条 prompt
- **THEN** 系统 SHALL 创建正式 session、提交请求并持续渲染流式回复
- **AND** 回复结束后该会话 MUST 可继续接受后续 prompt

#### Scenario: 恢复并切换现有会话

- **WHEN** 用户从终端选择一个已有 session 进行恢复或从当前会话切换到另一个 session
- **THEN** 终端客户端 SHALL 基于服务端读模型恢复该 session 的当前 transcript 与执行状态
- **AND** MUST NOT 依赖当前终端进程的本地内存来重建历史状态

#### Scenario: 恢复时目标会话不存在

- **WHEN** 用户尝试恢复一个已被删除、不可访问或不存在的 session
- **THEN** 系统 SHALL 返回明确的 not found / forbidden 结果
- **AND** MUST NOT 把用户静默带回错误的会话

#### Scenario: v1 仅维护一个 active session 的 live stream

- **WHEN** 用户在终端切换到另一个 session
- **THEN** 系统 SHALL 将 live stream 焦点切换到新的 active session
- **AND** v1 MUST NOT 同时维护多个 session 的并行 live stream

### Requirement: 终端客户端 SHALL 提供完整 slash command 交互

终端客户端 MUST 以键盘驱动方式提供 `/new`、`/resume`、`/compact`、`/skill` 等正式 slash command 体验，并把命令交互映射到稳定的服务端合同，而不是在本地实现平行业务语义。

#### Scenario: 执行 `/new`

- **WHEN** 用户在终端输入并确认 `/new`
- **THEN** 系统 SHALL 创建新 session 并把输入焦点切换到该 session
- **AND** 旧 session 的 transcript 与执行状态 MUST 仍可通过恢复流程访问

#### Scenario: 执行 `/resume`

- **WHEN** 用户在终端输入 `/resume` 并选择一个候选 session
- **THEN** 系统 SHALL 切换到所选 session 并渲染该 session 的最新状态
- **AND** 候选列表 MUST 反映服务端提供的正式会话事实

#### Scenario: 执行 `/skill`

- **WHEN** 用户在终端输入 `/skill` 并选择一个 skill 候选
- **THEN** 系统 SHALL 使用 discovery 返回的正式候选元数据完成插入或触发后续提交流程
- **AND** MUST NOT 依赖 CLI 本地硬编码一份独立 skill 注册表

#### Scenario: 输入无效 slash command

- **WHEN** 用户提交一个不存在、当前上下文不可用或参数不合法的 slash command
- **THEN** 终端客户端 SHALL 返回明确错误或校验提示
- **AND** MUST NOT 把该命令静默降级为普通消息提交

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

### Requirement: 终端客户端 SHALL 在高吞吐与能力受限环境下保持可用

终端客户端 MUST 在高 token 速率、窗口 resize 和终端能力受限时继续保持可用，而不是要求所有宿主都具备完整 truecolor / alt-screen / unicode 支持。

#### Scenario: 流式输出进入 catch-up 模式

- **WHEN** 流式 chunk 积压导致渲染跟不上实时输出
- **THEN** 终端客户端 SHALL 能切换到面向追赶的渲染策略
- **AND** MUST 避免因为无限缓冲而失去响应

#### Scenario: 窗口 resize 触发布局重算

- **WHEN** 终端窗口宽度或高度发生变化
- **THEN** 终端客户端 SHALL 重算换行、滚动锚点与 pane 布局
- **AND** MUST NOT 继续复用已失效的旧布局缓存

#### Scenario: 终端能力不足时优雅降级

- **WHEN** 当前终端环境不支持 truecolor、alt-screen、mouse 或完整 unicode 宽度
- **THEN** 终端客户端 SHALL 退化到兼容渲染模式
- **AND** MUST NOT 因能力缺失而拒绝提供基本聊天工作流
