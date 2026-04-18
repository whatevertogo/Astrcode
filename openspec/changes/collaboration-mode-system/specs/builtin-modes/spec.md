## ADDED Requirements

### Requirement: BuiltinModeCatalog 注册三个预定义模式
系统 SHALL 提供 `BuiltinModeCatalog` 实现，注册以下三个 ModeSpec：

**Plan 模式：**
- tool_grants: SideEffect(None) + Named("toolSearch")
- system_directive: 只读分析、结构化方案、不修改代码
- entry_policy: LlmCanEnter
- transitions: → execute (requires_confirmation=true), → review (requires_confirmation=false)
- output_artifact_kind: Some("plan")

**Execute 模式：**
- tool_grants: All
- system_directive: 完整执行权限
- entry_policy: 默认模式
- transitions: → plan (requires_confirmation=false), → review (requires_confirmation=false)
- output_artifact_kind: None

**Review 模式：**
- tool_grants: SideEffect(None) + Named("toolSearch")
- system_directive: 代码审查、质量检查
- entry_policy: LlmCanEnter
- transitions: → execute (requires_confirmation=true), → plan (requires_confirmation=false)
- output_artifact_kind: Some("review")

#### Scenario: list_modes 返回三个模式
- **WHEN** BuiltinModeCatalog.list_modes() 被调用
- **THEN** 返回包含 plan、execute、review 三个 ModeSpec 的列表

#### Scenario: resolve_mode 找到指定模式
- **WHEN** BuiltinModeCatalog.resolve_mode("plan") 被调用
- **THEN** 返回 Some(ModeSpec { id: "plan", ... })

#### Scenario: resolve_mode 找不到不存在的模式
- **WHEN** BuiltinModeCatalog.resolve_mode("nonexistent") 被调用
- **THEN** 返回 None
