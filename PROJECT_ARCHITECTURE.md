# Astrcode 架构约束

本文档是当前仓库的权威架构说明。目标不是解释历史，而是约束未来实现。

## 总原则

- 不维护向后兼容，优先最终边界和干净代码。
- `server` 是唯一组合根，只负责装配，不承载长期业务真相。
- `core` 只保留真正跨 owner 共享的稳定语义，不再充当 DTO/trait 总仓库。
- runtime、session host、plugin host 分离，避免“大核心 + 补丁式扩展”。
- 多 agent 协作继续遵循“一个 session 即一个 agent”，durable truth 统一归 `host-session`。

## 目标分层

### `astrcode-core`

只保留跨 owner 共享的稳定值对象和最小合同：

- `ids`
- LLM / tool / message 基础模型
- `CapabilitySpec`
- 极少数共享 prompt 语义
- hooks 的稳定事件键和 effect kind

不得继续承载：

- session recovery / projection / read model
- workflow / mode / session catalog
- plugin registry / active snapshot / plugin manifest
- owner 专属 config / observability / store / composer / ports
- 多 agent 协作 durable truth

### `astrcode-agent-runtime`

最小执行内核，只负责单 turn / 单 agent live 执行：

- `execute_turn`
- provider stream 调用
- tool dispatch
- hook dispatch
- 流式状态机
- 取消 / 超时传播

不得负责：

- session catalog
- 事件日志与恢复
- branch / fork / compact
- resource discovery
- 多 agent 协作 durable truth

### `astrcode-host-session`

session owner，统一承接 durable truth 和 host use-case：

- 事件日志
- 恢复与回放
- projection / query / observe
- session catalog
- branch / fork / compact
- 模型选择
- 输入入口
- `AgentRuntimeExecutionSurface` 组装
- 多 agent 协作真相：`SubRunHandle`、`InputQueueProjection`、父子 lineage、结果投递、取消传播

### `astrcode-plugin-host`

统一 builtin / external plugin 宿主：

- plugin descriptor 校验
- candidate / active snapshot
- reload commit / rollback
- hooks / providers / resources / commands / prompts / skills / themes 聚合
- builtin backend 与 external backend 的统一语义

### `astrcode-server`

唯一组合根：

- 装配 `agent-runtime`、`host-session`、`plugin-host`
- 装配 `adapter-*`
- 暴露 HTTP / RPC / CLI 所需入口

不得继续承载：

- builtin / plugin / MCP / governance / workflow / mode 的并列事实源
- provider kind 硬编码选择逻辑
- 旧运行时协调壳层

## 迁移中的旧边界

以下 crate 已不再是长期权威边界，只允许作为迁移源存在，最终必须删除：

- `astrcode-application`
- `astrcode-kernel`
- `astrcode-session-runtime`
- 旧 `astrcode-plugin`

要求：

- 新 crate 不得回头依赖这些旧边界。
- 新功能不得继续落在这些 crate 中。
- 组合根不得继续把这些 crate 当成正式装配主链。

## 依赖方向

允许的高层方向如下：

```text
adapter-* ───────────────┐
                         ├──> plugin-host ──┐
storage / protocol ──────┘                  │
                                            │
core <──────────── agent-runtime <──────────┤
  ^                    ^                    │
  |                    |                    │
  └──────────── host-session <──────────────┘
                        ^
                        |
                      server
```

### 强约束

- `core` 不得依赖任何其他工作区 crate。
- `protocol` 仅允许依赖 `core`。
- `support` 仅允许依赖 `core`。
- `agent-runtime` 仅允许依赖 `core`，必要时可依赖极少数纯工具 crate；不得依赖 `application`、`kernel`、`session-runtime`。
- `plugin-host` 仅允许依赖 `core`、`protocol`、`support`；不得依赖 `application`、`kernel`、`session-runtime`。
- `host-session` 仅允许依赖 `core`、`support`、`agent-runtime`、`plugin-host`；不得依赖 `application`、`kernel`、`session-runtime`。
- `server` 是唯一允许同时装配新旧边界的地方，但目标是逐步只装配 `agent-runtime + host-session + plugin-host + adapters`。

## 多 agent 协作约束

- 一个 session 就是一个 agent。
- child agent 必须表现为 child session，而不是同一 session 内的“子人格切换”。
- `host-session` 是 collaboration durable truth 的唯一 owner。
- `agent-runtime` 只保留 child session 的最小执行合同。
- `plugin-host` 只暴露协作 surface，例如 `spawn_agent`、`send_to_child`、`send_to_parent`、`observe_subtree`、`terminate_subtree`；这些 surface 不得持有 durable truth。

## hooks 约束

- hooks 是唯一扩展总线。
- governance、workflow overlay、tool policy、resource discovery、model selection 必须逐步统一到 hooks catalog。
- hook effect 只能表达受约束的流程影响，不能直接突变 durable truth。
- prompt augment 必须继续走 `PromptDeclaration` / `PromptGovernanceContext` 链路，不引入平行 prompt 系统。

## 实施顺序

1. 先更新本文档和 crate boundary 守卫。
2. 新建 `agent-runtime`、`host-session`、`plugin-host` crate 骨架。
3. 收缩 `core`，先删共享面污染，再迁 owner 专属模型。
4. 迁移最小 runtime 核心到 `agent-runtime`。
5. 迁移 session durable truth 和协作真相到 `host-session`。
6. 迁移 plugin 宿主与统一 snapshot 到 `plugin-host`。
7. 重写 `server` 组合根。
8. 删除 `application`、`kernel`、旧 `session-runtime`、旧 `plugin` 边界。

## 验证要求

每次涉及边界变更时，至少验证：

- `node scripts/check-crate-boundaries.mjs`
- `cargo check --workspace`

进入大规模迁移后，再逐步补齐：

- `cargo test --workspace --exclude astrcode --lib`
- 新 crate 的单元测试与集成测试
