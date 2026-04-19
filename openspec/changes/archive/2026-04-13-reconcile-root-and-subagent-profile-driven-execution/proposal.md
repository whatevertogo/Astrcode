## Why

当前 root agent 执行入口与 subagent 执行入口都没有稳定地以已解析的 agent profile 作为事实源，导致 HTTP 路由、`application` 用例与既有 specs 之间出现明显漂移。这个问题已经影响到 profile 存在性校验、mode 校验、工具约束与模型偏好等核心语义，必须先收口执行入口，后续的 delivery、watch、observability 才有可靠基础。

## What Changes

- 统一 root agent 与 subagent 的执行入口语义，使两者都通过 `application` 的正式用例边界消费已解析 profile，而不是在路由层或编排层临时拼装最小 profile。
- **BREAKING** 收紧 root 执行合同：`POST /api/v1/agents/{id}/execute` 不再接受任意 `agent_id` 并直接创建 session 提交 prompt，而是要求目标 profile 可解析且允许作为根执行。
- **BREAKING** 收紧 subagent 执行合同：`spawn` 不再仅根据 `type` 字符串构造占位 profile，而是要求在 working-dir 作用域内解析真实 profile，并显式校验 mode、存在性与执行约束。
- 让 profile 解析服务进入真实执行主链，恢复 `system_prompt`、`allowed_tools`、`disallowed_tools`、`model_preference` 等 profile 语义对 root/subagent 执行的正式影响。
- 明确迁移边界：server 路由只负责协议转换与错误映射，不再绕过 `application` 正式执行入口；`kernel` 继续只提供控制合同，不承担 profile 选择。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `root-agent-execution`: 根执行必须通过正式 `application` 入口消费已解析 profile，并在 profile 不存在或 mode 不允许时显式失败。
- `subagent-execution`: 子代理执行必须基于 working-dir 解析真实 profile，并正式消费 execution control 与 profile 约束。
- `agent-profile-resolution`: profile 解析与缓存不再停留在孤立服务层，执行入口必须将其作为正式事实源使用。
- `agent-execution`: root/subagent 执行的稳定行为需要与 profile 驱动模型重新对齐，不再允许路由层与编排层各自持有不同语义。

## Impact

- 影响代码：
  - `crates/server/src/http/routes/agents.rs`
  - `crates/application/src/execution/root.rs`
  - `crates/application/src/agent/mod.rs`
  - `crates/application/src/execution/profiles.rs`
  - `crates/application/src/lib.rs`
- 影响接口：
  - `POST /api/v1/agents/{id}/execute`
  - 四工具中的 `spawn` 语义与错误返回
- 影响系统：
  - root agent 注册与控制树一致性
  - subagent profile 事实源
  - 后续 watch invalidation 与 child delivery pipeline 的接线前提

## Non-Goals

- 本次不处理 child delivery 到 parent wake 的回流链路；该内容由独立 change 收口。
- 本次不引入新的 agent profile 文件格式，也不调整 adapter-agents 的磁盘发现规则。
- 本次不处理 profile 文件变更监听与热失效；watch/invalidation 由独立 change 负责。

## Migration And Rollback

- 迁移方式为“入口收口优先”：先让 server 路由统一委托到 `application` 正式执行入口，再移除编排层中的占位 profile 构造逻辑。
- 回滚策略保持简单：若新执行入口在集成阶段暴露阻断性问题，可以临时恢复旧路由委托路径，但必须同时撤回对应 spec 变更，避免规范与实现再次漂移。
