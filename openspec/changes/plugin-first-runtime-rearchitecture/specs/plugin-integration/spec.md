## ADDED Requirements

### Requirement: plugin integration SHALL 基于统一 descriptor 与 snapshot 装配

系统 MUST 以统一 plugin descriptor 和 active snapshot 模型装配 plugin 贡献，而不是只把 plugin 视为 capability invoker 或窄版 hook 的补充来源。

#### Scenario: plugin 装配覆盖完整贡献面
- **WHEN** 宿主装载一个 plugin
- **THEN** 系统 SHALL 同时解析其 tool、hook、provider、resource、command、theme、prompt、skill 等贡献
- **AND** SHALL 在一次统一装配流程中完成校验与注册

### Requirement: plugin integration SHALL 不再只依赖 HookHandler 适配

plugin hook 集成 MUST 基于统一 hooks 平台接入，而不是继续把 plugin hook 视为 `core::HookHandler` 的薄适配层。

#### Scenario: plugin hook 进入统一 hooks registry
- **WHEN** 某个 plugin 声明 lifecycle hooks
- **THEN** 其 handlers SHALL 进入统一 hooks registry
- **AND** SHALL 与 builtin hooks 共享相同的事件、effect 与执行语义

### Requirement: plugin reload SHALL 以候选 surface commit/rollback 方式完成

plugin 集成 MUST 使用候选 surface 构建、校验、commit/rollback 的方式进行 reload，而不是边发现边直接修改当前生效 surface。

#### Scenario: reload 失败不污染当前 surface
- **WHEN** 某次 plugin reload 在校验或装配阶段失败
- **THEN** 当前 active surface SHALL 保持不变
- **AND** 系统 SHALL 报告失败原因

### Requirement: collaboration capabilities SHALL 通过统一 plugin surface 接入

多 agent 协作相关能力 MUST 通过统一 plugin surface 接入，而不是继续保留 `application` 或 `server` 私有特判入口。协作 surface 可以是 tool、command 或其他 plugin contribution，但它们 SHALL 只调用 `host-session` 的正式 use-case surface。

#### Scenario: 协作入口从 plugin surface 调用 host-session
- **WHEN** 某个 builtin collaboration tool 或 command 需要启动 child session、向 child 发送消息或终止子树
- **THEN** 该入口 SHALL 通过统一 plugin integration 进入 active snapshot
- **AND** SHALL 通过 `host-session` 完成 durable truth 写入与后续 runtime 驱动
