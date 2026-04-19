## Requirements

### Requirement: 工具发现通过 ComposerService 和 Capability Surface

server SHALL 通过 `ComposerService` 的 `list_options` 方法支持按名称、描述或关键字过滤已注册的工具和能力。

#### Scenario: 按关键字过滤工具

- **WHEN** `ComposerOptionsRequest` 携带 `query` 参数
- **THEN** `ComposerService.list_options` 返回 id、title、description 或 keywords 包含查询关键词的候选项
- **AND** 结果按 kind 过滤和 limit 截断

#### Scenario: 无匹配结果

- **WHEN** 搜索关键词不匹配任何工具或能力
- **THEN** 返回空列表

---

### Requirement: Skill 工具按需加载

adapter-tools SHALL 提供 `Skill` 工具（`SKILL_TOOL_NAME = "Skill"`），允许 agent 从 `SkillCatalog` 按需加载 skill 的完整指令和资源路径。

#### Scenario: 加载 skill

- **WHEN** agent 调用 `Skill` 工具并指定 `skill` 名称（kebab-case）
- **THEN** 系统从 `SkillCatalog` 查找匹配的 skill，返回其完整内容

#### Scenario: skill 不存在

- **WHEN** 指定的 skill 名称不存在
- **THEN** 返回错误信息列出可用的 skill

---

### Requirement: 外部工具目录

server SHALL 支持从 plugin 和 MCP server 动态注册外部工具。

#### Scenario: 注册 MCP 工具

- **WHEN** MCP server 连接并暴露工具列表
- **THEN** 工具被注册到 kernel 的 capability surface，对所有 session 可见

#### Scenario: MCP server 断开

- **WHEN** MCP server 断开连接
- **THEN** 其贡献的工具从 capability surface 移除

---

### Requirement: 配置连接测试

application `ConfigService` SHALL 支持测试 LLM provider 的连接是否正常。

#### Scenario: 测试成功

- **WHEN** 调用 `test_connection(profile_name, model)` 且 profile 存在且 model 在该 profile 中已配置
- **THEN** 返回 `TestConnectionResult { success: true, provider, model, error: None }`

#### Scenario: 测试失败

- **WHEN** 调用 `test_connection` 且 profile 不存在或 model 未配置
- **THEN** 返回 `TestConnectionResult { success: false, provider, model, error: Some(...) }`
