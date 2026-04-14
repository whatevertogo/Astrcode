## Why

Astrcode 的 agent-tool 底座已经比较健康：`spawn / send / observe / close` 的职责边界、`direct-parent + durable mailbox` 的执行模型，以及 launch-time `resolved capability surface` 都已经明确。当前真正欠缺的不是更多 runtime 形态，而是更符合现有架构的 delegation experience：模型能看到哪些行为模板、怎样区分 fresh / resumed / restricted 三种 child、以及 child 完成后如何让父级快速做出下一步决策，这几层还没有被系统化。

如果直接照搬 Claude Code 的 agent 列表和工具配置心智，会和 Astrcode 现有设计冲突，因为本项目已经明确把 `AgentProfile` 收敛成行为模板，而不是 capability 权限真相。现在需要补的是一套贴合 Astrcode 架构的 prompt/delegation contract：通用协作协议、行为模板目录、child 专属执行合同，以及更强的观察结果投影。

## What Changes

- 新增 `agent-delegation-surface` capability，正式定义模型可见的 delegation prompt surface：行为模板目录、child execution contract，以及它们与现有 prompt contributor 分层的关系。
- 修改 `agent-tool-governance` capability，要求通用协作协议继续由共享 guidance 承担，并明确区分 fresh child、resumed child、restricted child 三种 delegation mode；各工具 description 保持低噪音和动作导向。
- 修改 `subagent-execution` capability，要求 child 执行与观察结果暴露 responsibility continuity 与 advisory 决策线索，帮助父级判断是复用当前 child、关闭当前分支，还是在 capability / responsibility mismatch 时创建新的 child。
- 明确行为模板目录不得伪装成 capability 授权目录：profile 继续表示“怎么做事”，真正的 capability truth 仍由 runtime 在 launch 时求解。

## Capabilities

### New Capabilities

- `agent-delegation-surface`: 定义模型可见的 delegation prompt surface，包括行为模板目录、child execution contract 与 child-scoped prompt 注入边界。

### Modified Capabilities

- `agent-tool-governance`: 协作工具 guidance 需要从“平铺的工具手册”升级成“围绕 delegation mode 的低噪音决策协议”，并明确共享 guidance 与工具 description 的分工。
- `subagent-execution`: child 执行与观察结果需要暴露 responsibility continuity、reuse / close / respawn 的 advisory 线索，并在 restricted child 场景下清楚表达 capability mismatch。

## Impact

- 影响代码：
  - `crates/adapter-prompt/src/contributors/{workflow_examples,agent_profile_summary,prompt_declaration}.rs`
  - `crates/adapter-prompt/src/{context,core_port,layered_builder}.rs`
  - `crates/server/src/bootstrap/prompt_facts.rs`
  - `crates/adapter-tools/src/agent_tools/{spawn_tool,send_tool,observe_tool,close_tool,collab_result_mapping}.rs`
  - `crates/application/src/execution/subagent.rs`
  - `crates/application/src/agent/routing.rs`
  - 如需把 advisory projection 暴露到调试/HTTP 面，再影响 `crates/server/src/http/*`
- 用户可见影响：
  - `spawn` 更容易写出符合当前 child mode 的 briefing
  - 模型会看到更准确的行为模板目录，而不是把 profile 误解成工具权限组合
  - child 更像“任务分支”，`observe` 结果也更容易直接支持下一步动作
- 开发者可见影响：
  - prompt contributor 的三层职责更清晰：共享协议、行为模板目录、child 专属合同
  - profile / capability grant / resolved surface 的边界不会被新的 prompt 设计打乱
  - 后续若继续做 workflow preset、background child 或更强 UI 状态面，会有更稳定的 delegation contract 可复用

## Non-Goals

- 不引入 Claude Code 的 background teammate / worktree isolation / remote fork 运行模型。
- 不把 `spawn / send / observe / close` 收敛成单个大而全的 agent RPC。
- 不让行为模板目录展示伪造的“per-profile tool list”或重新让 `AgentProfile` 承担 capability 授权职责。
- 不在本 change 中建设独立的效果埋点或 agent-tool analytics 系统。
