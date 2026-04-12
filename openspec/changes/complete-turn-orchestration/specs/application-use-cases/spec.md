## MODIFIED Requirements

### Requirement: `application` 负责用例编排、参数校验和权限前置

`application` SHALL 负责：

- 参数校验
- 权限前置检查
- 用例编排
- 业务错误归类

#### Scenario: 非法请求在 application 层被拒绝

- **WHEN** 传入无效 session id 或非法参数
- **THEN** `application` 直接返回业务错误
- **AND** 不将错误请求继续下推到 `kernel` 或 `session-runtime`

#### Scenario: submit_prompt 只触发 turn，不持有 turn 内策略

- **WHEN** `App::submit_prompt` 被调用
- **THEN** `application` 只负责校验输入、读取生效配置并调用 `SessionRuntime`
- **AND** token budget、continue nudge、turn 内 observability 不在 `application` 中实现
