## Purpose

定义 `core` crate 收缩边界的需求，将其限制为跨 owner 共享的最小语义层，不再作为所有 DTO、trait 和内部状态模型的总仓库。

## Requirements

### Requirement: `core` SHALL 只保留跨 owner 共享的最小语义

系统 MUST 将 `crates/core` 收缩为极薄的共享语义层。`core` SHALL 只保留多个 owner 共同消费且长期稳定的值对象、消息模型、能力语义与少量共享枚举，SHALL NOT 继续作为所有 DTO、trait 和内部状态模型的总仓库。

#### Scenario: 审查 core 导出面时只看到共享语义
- **WHEN** 审查 `crates/core/src/lib.rs` 的公开导出
- **THEN** 导出面 SHOULD 主要由 `ids`、消息模型、`CapabilitySpec`、最小 prompt/hook 共享语义组成
- **AND** SHALL NOT 再大量导出 owner 专属 checkpoint、projection、plugin registry、workflow/mode 或配置存储模型

### Requirement: owner 专属 DTO SHALL 跟随 owner crate

只被单一 owner 或单一边界使用的 DTO、快照、恢复模型、执行面、descriptor 与 observability 报告 MUST 迁回对应 owner crate，而不是继续放在 `core`。

#### Scenario: session 恢复和 projection 模型迁回 host-session
- **WHEN** 系统需要表达 session recovery checkpoint、projection snapshot、query/read model
- **THEN** 这些模型 SHALL 归属 `host-session`
- **AND** SHALL NOT 继续停留在 `core`

#### Scenario: plugin 描述模型迁回 plugin-host
- **WHEN** 系统需要表达 plugin descriptor、active snapshot、resource/theme/prompt/skill descriptor
- **THEN** 这些模型 SHALL 归属 `plugin-host`
- **AND** SHALL NOT 继续作为 `core` 的默认 DTO 集合

#### Scenario: runtime 执行面迁回 agent-runtime
- **WHEN** 系统需要表达 turn 执行面、runtime hook payload、provider/tool 执行上下文
- **THEN** 这些模型 SHALL 归属 `agent-runtime`
- **AND** SHALL NOT 继续通过 `core::ports` 之类的 mega 模块承载

### Requirement: `core` SHALL 删除膨胀型 mega 模块

`core` 中承载多 owner 合同和历史兼容模型的 mega 模块 MUST 被拆分或删除，至少包括 `ports.rs` 这类混合型合同文件，以及 `projection`、`workflow`、`plugin registry`、`session catalog` 等 owner 专属模块。`mode`、config、observability 需要先按共享合同与 owner 实现拆分：`ModeId`、durable mode-change event DTO、mode tool-contract snapshot、runtime config、observability wire metrics MAY 暂留 `core`，直到协议/事件 DTO 拆出稳定 wire schema；治理 DSL、builtin mode owner 逻辑、配置持久化、metrics collector 不得继续回流到 `core`。

#### Scenario: mega ports 被 owner 合同替代
- **WHEN** 需要定义 provider、prompt、resource、event-store 等合同
- **THEN** 它们 SHALL 被迁入对应 owner crate 或更小的专属合同模块
- **AND** SHALL NOT 继续集中堆叠在 `core::ports`

#### Scenario: mode/config/observability 先拆 owner 实现再拆共享 wire 合同
- **WHEN** 某个 `mode`、config 或 observability 类型仍被 durable event、`ToolContext`、session-runtime、server 和 protocol 同时消费
- **THEN** 它 MAY 暂留 `core` 作为共享 wire/control 合同
- **AND** 对应 owner 实现（治理 DSL 编译、builtin mode 装配、配置持久化、metrics collector）SHALL 迁入 `plugin-host`、`server`、`application` 或 `host-session`
- **AND** 后续只有在协议/事件 DTO 稳定拆分后，才能继续迁出这些共享合同

### Requirement: `core::ports` SHALL 按 owner 拆散

当前 `core::ports` 中混合的合同 MUST 按 owner 重新归属，至少满足：

- `EventStore`、`SessionRecoveryCheckpoint`、`RecoveredSessionState` -> `host-session`
- `LlmProvider` / provider stream 合同 -> `agent-runtime` 或专属 provider 合同模块
- `PromptProvider` / `PromptFactsProvider` -> `host-session`
- `ResourceProvider`、skill/prompt/theme/resource 发现合同 -> `plugin-host` 或 host 资源层

#### Scenario: 旧 mega ports 不再作为统一入口存在
- **WHEN** 实现者查找某个合同应放在哪个 crate
- **THEN** 可以根据 owner 直接定位到 `agent-runtime`、`host-session` 或 `plugin-host`
- **AND** SHALL NOT 再以 `core::ports` 作为默认落点

### Requirement: `core` SHALL 不以"纯数据"为理由继续吸纳模型

本次重构中，系统 SHALL NOT 仅因为某个结构体可序列化或看起来像 DTO，就将其放入 `core`。进入 `core` 的必要条件是"多个 owner 共同消费且语义长期稳定"，而不是"它是纯数据"。

#### Scenario: 可序列化但 owner 单一的模型不进入 core
- **WHEN** 某个模型只服务于 plugin reload、session recovery 或 runtime execution
- **THEN** 即使它是纯数据结构
- **AND** 它也 SHALL 归属对应 owner crate，而不是进入 `core`

### Requirement: collaboration lineage 与 input queue 模型 SHALL 迁出 `core`

当前 `core/src/agent/*` 中的 `SubRunHandle`、`InputQueueProjection`、协作 executor 合同与相关投影模型 MUST 迁出 `core` 的顶层共享面，并通过 `host-session` owner bridge 对新调用方暴露。其中 durable collaboration truth SHALL 归 `host-session`，最小执行合同若仍需要存在，也 SHALL 归对应执行 owner，而不是继续停留在共享层。

迁移期例外：`ChildAgentRef`、`ChildSessionNode`、`ChildSessionLineageKind` MAY 暂留 `core`，因为它们当前嵌入 `ChildSessionNotification` / `StorageEventPayload` 等 durable event DTO。除非先拆出稳定 wire schema，否则迁出这些类型会导致 `core` 反向依赖 `host-session` 或复制事件 DTO。

#### Scenario: 协作模型不再作为 core 默认导出面
- **WHEN** 审查重构后的 `crates/core/src/lib.rs`
- **THEN** 不应再把 sub-run lineage、input queue projection、协作读模型作为 `core` 默认导出
- **AND** 实现者应从 `host-session` 或运行时 owner 获取这些模型
