## Why

当前讨论中的方案倾向于让 `AgentProfile` 直接承载子 agent 的工具配置，但这会把“行为模板”和“运行时能力真相”混在一起。对 AstrCode 来说，能力语义的唯一事实源已经是 `CapabilitySpec` / capability router；如果再让 profile 成为工具授权中心，会把 capability system 弄成双真相。

更合适的方向是：profile 只描述 child 的行为风格与默认模型，而 `spawn` 在本次任务里显式申请 capability grant，runtime 再从父级可继承能力面、grant 与当前 capability router 求交得到 child 的 resolved capability surface。这样 prompt、执行、状态查询与 event log 都能围绕同一份能力真相工作。

## What Changes

- 将 `AgentProfile` 明确收敛为行为模板：主要负责 prompt 风格、模型偏好与协作默认值，不再作为 child 工具授权真相。
- 为 `spawn` 引入任务级 `capability grant` 概念，用于声明本次 child 任务需要的最小工具范围。
- 修改 subagent 执行流程，使 child 的 resolved capability surface 由 `parent inheritable surface ∩ spawn grant ∩ runtime availability` 求得，而不是由 profile 直接决定。
- 修改 child prompt 组装与 tool execution，使二者共同消费同一份 filtered capability router。
- 扩展 subrun durable/status 合同，记录并暴露 child 启动时的 resolved capability surface snapshot，便于 `observe`、status 与调试读取。
- 更新 `spawn` 的协作 guidance，使其明确区分“选 profile 是选行为模板”和“给 capability grant 是限定本次任务范围”。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `capability-semantic-model`: child capability surface 需要明确为 `CapabilitySpec` / capability router 的派生投影，不允许由 `AgentProfile` 成为第二事实源。
- `subagent-execution`: 子代理执行入口需要支持 task-scoped capability grant，并在 child 启动时求得 resolved capability surface。
- `subrun-status-contracts`: subrun 状态合同需要暴露 child 启动时的 resolved capability surface snapshot，保证 status 可解释。
- `agent-tool-governance`: `spawn` 等协作工具的 prompt guidance 需要明确 profile 与 capability grant 的角色分工，以及 capability mismatch 下的决策边界。

## Impact

- 影响代码：
  - `crates/core/src/agent/mod.rs`
  - `crates/application/src/execution`
  - `crates/application/src/agent`
  - `crates/kernel/src/registry/router.rs`
  - `crates/session-runtime/src/turn`
  - `crates/server/src/bootstrap/prompt_facts.rs`
  - `crates/server/src/http/routes/agents.rs`
  - `crates/adapter-tools/src/agent_tools/spawn_tool.rs`
- 影响协议：
  - `spawn` 请求参数
  - subrun status / event 中的 `resolved_limits` 与相关 DTO
- 影响用户可见行为：
  - child agent 的提示词与工具面将按本次任务授权收缩，行为更可预测
- 影响开发者：
  - 需要在 `spawn` 时显式考虑本次任务的 capability grant，而不是试图靠 profile 预组合所有权限情况

## Non-Goals

- 本次不引入 team / swarm 组织层。
- 本次不实现跨树通信或兄弟 agent 协作协议扩展。
- 本次不实现完整 fork/resume context inheritance 重构。
- 本次不重做 hook 系统或全局 agent-tool evaluation 读模型。
- 本次不把 policy engine 改造成新的 capability registry；审批语义继续留在现有 policy engine。
- 本次不实现 tag/permission/side-effect selector 的完整 grant 语言；第一版只要求可务实落地的 task-scoped capability grant。

## Migration And Rollback

- 迁移策略：保持现有 `spawn` 默认行为可用；若未显式传入 capability grant，则 child 先按父级可继承能力面与 runtime availability 启动。新能力以可选字段方式进入，不强制一次性迁移所有调用方。
- 回滚策略：若 capability grant / resolved surface 接线后出现 child 无法完成任务的回归，可先保留 status 中的 resolved snapshot，并临时回退到“不启用 task-scoped grant 收缩”的路径。
