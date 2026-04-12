## ADDED Requirements

### Requirement: 工具模糊搜索
server SHALL 支持按名称或描述模糊搜索已注册的工具。

#### Scenario: 按名称搜索
- **WHEN** 调用 `search_tools("read")`
- **THEN** 返回名称或描述中包含 "read" 的所有工具，按相关度排序

#### Scenario: 无匹配结果
- **WHEN** 搜索关键词不匹配任何工具
- **THEN** 返回空列表

### Requirement: Skill Tool 命令
adapter-tools SHALL 提供 `/skill` 工具，允许 agent 从 skill catalog 查找并执行 skill。

#### Scenario: 查找 skill
- **WHEN** agent 调用 skill tool 指定 skill 名称
- **THEN** 系统从 skill catalog 查找匹配的 skill，返回其内容

#### Scenario: skill 不存在
- **WHEN** 指定的 skill 名称不存在
- **THEN** 返回错误信息列出可用的 skill

### Requirement: 外部工具目录
server SHALL 支持从 plugin 和 MCP server 动态注册外部工具。

#### Scenario: 注册 MCP 工具
- **WHEN** MCP server 连接并暴露工具列表
- **THEN** 工具被注册到 kernel 的 capability surface，对所有 session 可见

#### Scenario: MCP server 断开
- **WHEN** MCP server 断开连接
- **THEN** 其贡献的工具从 capability surface 移除

### Requirement: 配置连接测试
application ConfigService SHALL 支持测试 LLM provider 的连接是否正常。

#### Scenario: 测试成功
- **WHEN** 调用 `test_connection(profile_name)` 且 provider 可达
- **THEN** 返回 `TestConnectionResult::Success` 含模型信息

#### Scenario: 测试失败
- **WHEN** 调用 `test_connection` 且 provider 不可达
- **THEN** 返回 `TestConnectionResult::Failed` 含错误原因
