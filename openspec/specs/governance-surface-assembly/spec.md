## Purpose

定义统一治理装配的顶层架构，确保所有 turn 入口共享同一治理面解析路径，治理包络是 turn-scoped 治理输入的 authoritative 来源。

## Requirements

### Requirement: all turn entrypoints SHALL resolve a shared governance surface before session-runtime submission

系统 MUST 在 root execution、普通 session prompt 提交、fresh child launch 与 resumed child submit 等所有 turn 入口上，先解析一个统一的治理包络，再把它交给 `session-runtime`。

#### Scenario: root execution uses the shared governance assembly path

- **WHEN** 系统发起一次 root agent execution
- **THEN** 它 SHALL 先解析统一治理包络
- **AND** SHALL NOT 直接在调用点手工拼接 scoped router、prompt declarations 与其他治理输入

#### Scenario: subagent launch uses the same governance surface shape

- **WHEN** 系统启动一个 fresh 或 resumed child session
- **THEN** 它 SHALL 通过相同的治理装配入口生成治理包络
- **AND** 输出形状 SHALL 与其他 turn 入口一致

### Requirement: governance surface SHALL be the authoritative source for turn-scoped governance inputs

治理包络 MUST 成为 turn-scoped capability router、prompt declarations、resolved limits、context inheritance 与 child contract 等治理输入的 authoritative 来源。

#### Scenario: session-runtime consumes a resolved governance envelope

- **WHEN** `session-runtime` 接收一次 turn 提交
- **THEN** 它 SHALL 读取已解析的治理包络作为治理输入
- **AND** SHALL NOT 在底层重新推导业务级治理决策

#### Scenario: prompt declarations come from the governance surface

- **WHEN** 当前 turn 需要内置协作 guidance、child contract 或其他治理声明
- **THEN** 这些声明 SHALL 来源于治理包络
- **AND** SHALL 通过统一的 `PromptDeclaration` 链路进入 prompt 组装

### Requirement: governance surface cleanup SHALL preserve current default behavior while removing duplicated assembly paths

本轮治理收口重构 MUST 以行为等价为默认目标；在没有显式新治理配置的前提下，root/session/subagent 入口的默认执行行为 SHALL 与当前保持等价。

#### Scenario: default execute path remains behaviorally equivalent

- **WHEN** 系统在未启用额外治理配置的情况下提交普通执行任务
- **THEN** 模型可见工具、默认协作 guidance 与 child contract 语义 SHALL 与当前默认行为保持等价

#### Scenario: duplicate assembly logic is removed without changing runtime engine

- **WHEN** 完成本轮 cleanup
- **THEN** turn 相关治理输入 SHALL 由统一装配路径生成
- **AND** `run_turn`、tool cycle、streaming path 与 compaction engine SHALL 保持单一实现

### Requirement: bootstrap governance assembly SHALL provide a clear entrypoint for mode system integration

`build_app_governance`（server/bootstrap/governance.rs:43-80）和 `GovernanceBuildInput` 是服务器级治理组合根。它们 MUST 为后续 mode system 提供明确的接入点，使 mode catalog 能在 bootstrap/reload 阶段被装配。

#### Scenario: GovernanceBuildInput exposes mode-catalog-ready assembly hooks

- **WHEN** 后续 governance mode system 需要在 bootstrap 阶段注册 mode catalog
- **THEN** `GovernanceBuildInput` SHALL 已具备接入 mode catalog 的参数或接口
- **AND** SHALL NOT 要求修改 bootstrap 编排流程的核心结构

#### Scenario: AppGovernance reload path supports mode catalog swap

- **WHEN** 运行时 reload 触发能力面和配置的原子替换
- **THEN** reload 编排 SHALL 能同时替换 mode catalog（如果存在）
- **AND** SHALL NOT 因缺少接入点而要求在 mode system 实现时重新编排 reload 流程

### Requirement: runtime governance lifecycle SHALL keep clear boundaries between governance assembly and runtime execution

`AppGovernance`（application/lifecycle/governance.rs）负责 reload/shutdown 生命周期管理。治理装配与运行时执行的边界 MUST 保持清晰，治理装配层不吞并 runtime engine 的执行控制逻辑。

#### Scenario: reload governance check remains in application layer

- **WHEN** `AppGovernance.reload()` 检查是否有 running session
- **THEN** 该检查 SHALL 继续在 application 层完成
- **AND** SHALL NOT 下沉到 session-runtime 或 kernel 层

#### Scenario: capability surface replacement uses the governance assembly path

- **WHEN** `RuntimeCoordinator.replace_runtime_surface` 执行原子化能力面替换
- **THEN** 新能力面 SHALL 通过治理装配路径传递到后续 turn 提交
- **AND** SHALL NOT 出现替换后的能力面与正在执行的 turn 使用的能力面不一致的竞态

### Requirement: CapabilitySurfaceSync and runtime coordinator SHALL be governance-surface-aware

`CapabilitySurfaceSync`（server/bootstrap/capabilities.rs:108-156）管理 stable local + dynamic external 能力面的同步。`RuntimeCoordinator`（core/runtime/coordinator.rs）负责原子化运行时表面替换。两者 MUST 在治理面变更后能通知治理装配器刷新缓存。

#### Scenario: capability surface change triggers governance envelope refresh

- **WHEN** MCP 连接变更或插件 reload 导致能力面发生改变
- **THEN** 后续 turn 的治理包络 SHALL 使用更新后的能力面
- **AND** SHALL NOT 使用 stale 的缓存能力面继续生成治理包络
