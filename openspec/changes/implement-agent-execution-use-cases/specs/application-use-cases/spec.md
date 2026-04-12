## MODIFIED Requirements

### Requirement: `application` 负责用例编排、参数校验和权限前置

`application` SHALL 负责：

- 参数校验
- 权限前置检查
- 用例编排
- 业务错误归类
- 根代理执行与子代理执行入口编排

#### Scenario: 非法请求在 application 层被拒绝

- **WHEN** 传入无效 session id 或非法参数
- **THEN** `application` 直接返回业务错误
- **AND** 不将错误请求继续下推到 `kernel` 或 `session-runtime`

#### Scenario: application 承接执行入口但不持有执行真相

- **WHEN** 发起根代理执行或子代理执行
- **THEN** `application` 负责解析 profile、校验输入、编排调用
- **AND** 单 session 执行真相仍由 `session-runtime` 持有
- **AND** 全局 agent control 真相仍由 `kernel` 持有
