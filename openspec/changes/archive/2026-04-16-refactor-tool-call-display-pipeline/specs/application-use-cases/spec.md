## MODIFIED Requirements

### Requirement: `application` 负责用例编排、参数校验和权限前置

`application` SHALL 负责：

- 参数校验
- 权限前置检查
- 用例编排
- 业务错误归类
- 根代理执行与子代理执行入口编排
- 跨 session 的父子协作编排

`application` MUST NOT 继续承载以下单 session 真相细节：

- 单 session 终态投影与轮询判定
- durable mailbox append 细节
- child/open session observe 快照拼装
- recoverable delivery 重放与投影细节
- conversation/tool display 的底层 transcript/replay 聚合细节

在 conversation/tool display 路径上，`application` MUST 通过 `SessionRuntime` 暴露的稳定 query/command 接口读取 authoritative read model facts，并把它们作为稳定用例结果返回给 `server`；`application` MUST NOT 再把 `SessionTranscriptSnapshot`、原始 replay receiver 或需要上层继续拼装的底层事实作为正式会话展示合同向上传递。

#### Scenario: 非法请求在 application 层被拒绝

- **WHEN** 传入无效 session id 或非法参数
- **THEN** `application` 直接返回业务错误
- **AND** 不将错误请求继续下推到 `kernel` 或 `session-runtime`

#### Scenario: submit_prompt 只触发 turn，不持有 turn 内策略

- **WHEN** `App::submit_prompt` 被调用
- **THEN** `application` 只负责校验输入、读取生效配置并调用 `SessionRuntime`
- **AND** token budget、continue nudge、turn 内 observability 不在 `application` 中实现

#### Scenario: application 承接执行入口但不持有执行真相

- **WHEN** 发起根代理执行或子代理执行
- **THEN** `application` 负责解析 profile、校验输入、编排调用
- **AND** 单 session 执行真相仍由 `session-runtime` 持有
- **AND** 全局 agent control 真相仍由 `kernel` 持有

#### Scenario: application 只通过 session-runtime 稳定接口读取单 session 细节

- **WHEN** `application` 需要判断 turn 终态、读取 observe 视图或追加 mailbox durable 事件
- **THEN** 统一通过 `SessionRuntime` 暴露的稳定 query/command 入口完成
- **AND** 不直接操作 `SessionState`、event replay 细节或投影组装过程

#### Scenario: conversation 工具展示用例返回稳定 facts

- **WHEN** `server` 请求 conversation snapshot、stream catch-up 或工具展示相关会话事实
- **THEN** `application` SHALL 返回已收敛的 conversation/tool display facts
- **AND** `server` 只负责 DTO 映射、HTTP 状态码与 SSE framing
- **AND** 前端消费合同 MUST NOT 依赖 `application` 之外的额外 transcript/replay 拼装
