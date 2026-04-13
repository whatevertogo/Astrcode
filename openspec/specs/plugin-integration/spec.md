## Requirements

### Requirement: 插件发现

server bootstrap SHALL 支持从多个路径发现并加载插件。

#### Scenario: 从配置路径发现插件

- **WHEN** bootstrap 阶段读取配置中的 plugin_paths
- **THEN** 系统扫描指定路径，发现所有 manifest.json/plugin.yaml 文件并加载

#### Scenario: 无配置路径

- **WHEN** 配置中没有指定 plugin_paths
- **THEN** 跳过插件发现阶段，不影响系统启动

---

### Requirement: Hook → Plugin 适配

server SHALL 将 core 的 HookHandler trait 适配为 plugin crate 的调用接口。

#### Scenario: 注册 Hook

- **WHEN** 插件声明了 PreToolUse 或 PreCompact hook
- **THEN** 系统创建 HookHandler 适配器，将 hook 调用转发到 plugin process

#### Scenario: Hook 执行

- **WHEN** turn 执行中触发 hook
- **THEN** 适配器通过 plugin 的 JSON-RPC peer 调用插件的 hook handler，返回结果

---

### Requirement: Skill 物化

server bootstrap SHALL 将 plugin 声明的 skill 转化为运行时可用的 CapabilitySpec。

#### Scenario: 物化 plugin skill

- **WHEN** 插件声明了 skill 资源
- **THEN** 系统将 skill 转化为 CapabilitySpec 并注册到 kernel 的 capability surface

---

### Requirement: Capability Surface 热重载

server SHALL 支持在不重启的情况下重新加载 capability surface（含 plugin 贡献的能力）。

#### Scenario: 触发热重载

- **WHEN** 配置文件变更或 plugin 状态变化
- **THEN** 系统重新组装 capability surface，更新 kernel 的 CapabilityRouter

#### Scenario: 热重载期间进行中的 turn

- **WHEN** 热重载触发时有 turn 正在执行
- **THEN** 进行中的 turn 继续使用旧的 surface，新 turn 使用新 surface
