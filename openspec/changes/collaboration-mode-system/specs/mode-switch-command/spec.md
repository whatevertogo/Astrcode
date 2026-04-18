## ADDED Requirements

### Requirement: /mode 命令切换协作模式
系统 SHALL 支持用户通过 `/mode <name>` 命令切换当前会话的协作模式。

#### Scenario: 用户成功切换到 plan 模式
- **WHEN** 用户输入 "/mode plan"
- **AND** 当前模式为 execute
- **THEN** session_mode 切换到 Plan
- **AND** 一条 ModeChanged 事件被持久化，source 为 User

#### Scenario: 用户切换到当前已处于的模式
- **WHEN** 用户输入 "/mode execute"
- **AND** 当前模式已经是 execute
- **THEN** 系统返回提示"已处于 execute 模式"，不产生事件

#### Scenario: 用户切换到不存在的模式
- **WHEN** 用户输入 "/mode nonexistent"
- **THEN** 系统返回错误"未知模式: nonexistent"

#### Scenario: /mode 不带参数显示当前模式
- **WHEN** 用户输入 "/mode" 不带参数
- **THEN** 系统返回当前模式名称和描述

### Requirement: /mode 命令绕过 entry_policy 检查
系统 SHALL 让用户通过 /mode 命令的切换不受 ModeEntryPolicy 限制。用户始终可以切换到任何可用模式。

#### Scenario: 用户直接切换到 UserOnly 模式
- **WHEN** 用户输入 "/mode plan"
- **AND** plan 模式的 entry_policy 是 UserOnly（假设）
- **THEN** 切换成功，因为用户手动操作绕过 entry_policy
