## Why

当前子代理链路已经具备 direct-parent + durable mailbox + wake 的稳定编排能力，但嵌套深度仍然过于隐式：

- runtime 默认深度散落在不同层，组合根没有统一把配置注入 kernel
- prompt 没有把“depth 是稀缺预算”明确告诉模型，导致模型更容易继续向下 spawn
- 当 depth 超限时，错误信息偏内部实现，不足以引导模型改用 `send / observe / close`

这会让多级协作在复杂任务下更容易膨胀，而不是像 Claude Code 那样优先复用 idle teammate。

## What Changes

- 将子代理最大嵌套深度变为显式 runtime 配置，并把默认值统一为 `3`
- 在 server 组合根把 `runtime.agent.max_subrun_depth` 注入 `KernelBuilder`
- 在 prompt facts 中暴露当前 depth limit，并在协作 guidance 中强调：
  - `Idle` 是正常状态
  - 优先复用已有 child，而不是继续向下 spawn
  - 命中 depth limit 后不要反复重试嵌套 spawn
- 改进 spawn 失败时的 application 错误映射，让 depth/concurrency 超限返回可执行建议

## Capabilities

### Modified Capabilities

- `subagent-execution`: 子代理最大嵌套深度必须可配置，默认值为 3，并在 runtime 与 prompt 侧保持一致

## Impact

- 影响代码：
  - `crates/application/src/config/constants.rs`
  - `crates/kernel/src/agent_tree/mod.rs`
  - `crates/server/src/bootstrap/runtime.rs`
  - `crates/server/src/bootstrap/prompt_facts.rs`
  - `crates/adapter-prompt/src/core_port.rs`
  - `crates/adapter-prompt/src/contributors/workflow_examples.rs`
  - `crates/adapter-tools/src/agent_tools/spawn_tool.rs`
  - `crates/application/src/execution/subagent.rs`
- 不修改外部 API / protocol DTO 结构

## Non-Goals

- 不改变现有 direct-parent + wake 的协作架构
- 不引入树级聚合或“等待整棵后代子树 settled”语义
- 不通过 prompt 替代 runtime 硬限制
