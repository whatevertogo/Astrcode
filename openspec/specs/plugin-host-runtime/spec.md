## Purpose

`plugin-host` 作为统一 builtin 与 external plugin 的能力层，用同一套 registry、descriptor、active snapshot 与 reload 语义承载所有扩展来源，消除"server 内置特判路径"和"plugin 补充路径"两套事实源。

## Requirements

### Requirement: `plugin-host` SHALL 统一 builtin 与 external plugin 的贡献模型

系统 MUST 新建统一的 `plugin-host` 能力层，用同一套 registry、descriptor、active snapshot 与 reload 语义承载 builtin 与 external plugin。系统 SHALL NOT 继续分别维护"server 内置特判路径"和"plugin 补充路径"两套事实源。

#### Scenario: builtin 与 external 进入同一 active snapshot
- **WHEN** 系统装配当前可用扩展面
- **THEN** builtin 与 external plugin 的贡献 SHALL 一起进入同一 active snapshot
- **AND** 新 turn SHALL 只消费这一个 snapshot

### Requirement: 统一 plugin descriptor SHALL 覆盖完整贡献面

统一 plugin descriptor MUST 至少支持以下贡献类型：`tools`、`hooks`、`providers`、`resources`、`commands`、`themes`、`prompts`、`skills`。系统 SHALL NOT 再把 commands、themes、prompts、skills 视为 plugin host 之外的平行体系。

#### Scenario: 一个 plugin 可同时贡献多类资源
- **WHEN** 某个 plugin 同时提供 tool、prompt、skill 与 theme
- **THEN** 它 SHALL 通过同一 plugin descriptor 描述这些贡献
- **AND** 宿主 SHALL 在同一发现/校验/装配流程中处理它们

### Requirement: `plugin-host` SHALL 支持进程内 builtin plugin 与外部 plugin 并存

为保证热路径性能和统一扩展面，`plugin-host` MUST 同时支持进程内 builtin plugin 与外部 plugin。两者共享同一 descriptor 与 snapshot 语义，但 MAY 使用不同执行后端。

#### Scenario: 热路径 builtin plugin 以内联方式运行
- **WHEN** 一个需要低延迟的 hooks 或工具贡献由 builtin plugin 提供
- **THEN** 系统 SHALL 允许其以内联方式执行
- **AND** SHALL NOT 强制其经过外部进程 hop

#### Scenario: 外部 plugin 继续通过隔离执行后端运行
- **WHEN** 一个 external plugin 被装载
- **THEN** 系统 MAY 通过进程、命令或远程协议运行它
- **AND** 它对上层暴露的 descriptor/snapshot 语义 SHALL 与 builtin plugin 保持一致

### Requirement: `plugin-host` SHALL 提供 snapshot 一致性的 reload 语义

`plugin-host` MUST 在 reload 时构建新的候选 snapshot，并以显式 commit/rollback 方式替换当前 active snapshot。进行中的 turn SHALL 继续使用旧 snapshot，新 turn SHALL 使用新 snapshot。

#### Scenario: reload 不打断进行中的 turn
- **WHEN** reload 发生时已有 turn 正在执行
- **THEN** 当前 turn SHALL 继续使用旧 snapshot 完成
- **AND** reload 成功后新 turn SHALL 切换到新 snapshot

#### Scenario: 候选 snapshot 构建失败时回滚
- **WHEN** 某个 plugin descriptor 校验失败、资源冲突或装配失败
- **THEN** 新候选 snapshot SHALL 被放弃
- **AND** 系统 SHALL 保持旧 active snapshot 不变

### Requirement: `plugin-host` SHALL 统一承接 provider contributions

`plugin-host` MUST 把 provider 视为正式 plugin contribution，而不是继续由 `server/bootstrap` 直接硬编码选择。provider 的发现、注册、校验、优先级与 active snapshot 集合 SHALL 统一进入 `plugin-host`。

#### Scenario: provider 与 tools/hooks 一起进入统一快照
- **WHEN** 某个 builtin 或 external plugin 贡献 provider
- **THEN** 它 SHALL 与该 plugin 的 tools/hooks/resources 一起进入统一 descriptor 与 active snapshot
- **AND** SHALL NOT 需要额外的平行 provider 装配路径

### Requirement: `plugin-host` SHALL 让新增 provider 不再要求修改 server 组合根

新增 provider 后端时，系统 SHOULD 只需要新增 provider backend 或 plugin contribution，而不是继续修改 `server/src/bootstrap/providers.rs` 的硬编码分支。

#### Scenario: 新 provider 通过 contribution 接入
- **WHEN** 系统新增一个非 OpenAI 的 provider backend
- **THEN** 它 SHOULD 通过 provider contribution / registry 接入
- **AND** `server` SHALL 不需要因为支持这个 provider 而再新增一条长期硬编码装配分支

### Requirement: collaboration surfaces MAY 作为 plugin contribution 暴露，但 SHALL 委托给 `host-session`

多 agent 协作相关的 tools/commands MAY 通过 builtin 或 external plugin contribution 进入统一 active snapshot，例如 `spawn_agent`、`send_to_child`、`send_to_parent`、`observe_subtree`、`terminate_subtree`。但这些 surface SHALL 只负责把动作提交给 `host-session`，而 SHALL NOT 自己持有 child session、sub-run lineage、input queue 或结果投递的 durable truth。

#### Scenario: 协作工具通过 plugin-host 暴露、通过 host-session 生效
- **WHEN** 当前 active snapshot 中存在 `spawn_agent` 之类的协作工具或命令
- **THEN** 它 SHALL 作为普通 plugin contribution 被发现、装配和暴露
- **AND** 实际 child session 创建与协作状态落库 SHALL 由 `host-session` 执行
