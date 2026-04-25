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

---

### Requirement: plugin integration SHALL 基于统一 descriptor 与 snapshot 装配

系统 MUST 以统一 plugin descriptor 和 active snapshot 模型装配 plugin 贡献，而不是只把 plugin 视为 capability invoker 或窄版 hook 的补充来源。

#### Scenario: plugin 装配覆盖完整贡献面
- **WHEN** 宿主装载一个 plugin
- **THEN** 系统 SHALL 同时解析其 tool、hook、provider、resource、command、theme、prompt、skill 等贡献
- **AND** SHALL 在一次统一装配流程中完成校验与注册

---

### Requirement: plugin integration SHALL 不再只依赖 HookHandler 适配

plugin hook 集成 MUST 基于统一 hooks 平台接入，而不是继续把 plugin hook 视为 `core::HookHandler` 的薄适配层。

#### Scenario: plugin hook 进入统一 hooks registry
- **WHEN** 某个 plugin 声明 lifecycle hooks
- **THEN** 其 handlers SHALL 进入统一 hooks registry
- **AND** SHALL 与 builtin hooks 共享相同的事件、effect 与执行语义

---

### Requirement: plugin reload SHALL 以候选 surface commit/rollback 方式完成

plugin 集成 MUST 使用候选 surface 构建、校验、commit/rollback 的方式进行 reload，而不是边发现边直接修改当前生效 surface。

#### Scenario: reload 失败不污染当前 surface
- **WHEN** 某次 plugin reload 在校验或装配阶段失败
- **THEN** 当前 active surface SHALL 保持不变
- **AND** 系统 SHALL 报告失败原因

---

### Requirement: collaboration capabilities SHALL 通过统一 plugin surface 接入

多 agent 协作相关能力 MUST 通过统一 plugin surface 接入，而不是继续保留 `application` 或 `server` 私有特判入口。协作 surface 可以是 tool、command 或其他 plugin contribution，但它们 SHALL 只调用 `host-session` 的正式 use-case surface。

#### Scenario: 协作入口从 plugin surface 调用 host-session
- **WHEN** 某个 builtin collaboration tool 或 command 需要启动 child session、向 child 发送消息或终止子树
- **THEN** 该入口 SHALL 通过统一 plugin integration 进入 active snapshot
- **AND** SHALL 通过 `host-session` 完成 durable truth 写入与后续 runtime 驱动
