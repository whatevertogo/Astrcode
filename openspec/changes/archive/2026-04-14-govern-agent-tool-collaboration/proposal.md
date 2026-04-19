## Why

当前 Astrcode 已经具备 `direct-parent + durable mailbox + wake + four tools` 的子代理基础设施，但模型侧仍然缺少一套正式的协作协议：`spawn` 容易被当成默认动作，`observe` 容易被当成轮询，`send` 与 `close` 的边界也还不够清晰。Claude Code 的经典之处不在于具体 runtime 形态，而在于它把 agent 协作建模成“任务分派后的下一步决策协议”；Astrcode 现在需要把这种协议正式落到 prompt、工具契约和运行时结果上。

## What Changes

- 新增 `agent-tool-governance` capability，正式定义 `spawn / send / observe / close` 的协作协议、提示词契约和最小必要结果语义。
- 修改 `subagent-execution` capability，要求 `send / observe / close` 成为直接父子关系下的正式业务入口，并补齐 direct-child 所有权、排队语义与失败路径行为。
- 将协作指导拆成系统级规则和工具级规则两层：系统级负责“何时用哪个工具”，工具级只保留最小动作约束，减少无用信息和无用思考。
- 让 `observe` 返回更适合下一步决策的结果投影，帮助主代理在“继续等待 / send 跟进 / close 分支”之间做选择，而不是反复轮询状态。
- 在不改变当前 child session 持久化模型的前提下，统一 child 所有权与展示语义，避免把 child agent 当成普通顶层会话心智使用。

## Capabilities

### New Capabilities

- `agent-tool-governance`: 定义 agent-tool 的四工具协作协议、提示词治理边界、最小结果契约以及与 Claude Code 风格对齐的使用原则。

### Modified Capabilities

- `subagent-execution`: 子代理的 `send / observe / close` 必须具备清晰的 direct-child 所有权、排队与终态语义，并为上级代理提供稳定的决策输入。

## Impact

- 影响代码：
  - `crates/adapter-tools/src/agent_tools/*`
  - `crates/adapter-prompt/src/contributors/workflow_examples.rs`
  - `crates/adapter-prompt/src/core_port.rs`
  - `crates/application/src/agent/{mod,routing,observe}.rs`
  - `crates/application/src/execution/subagent.rs`
  - `frontend/src/hooks/app/*` 与 `frontend/src/store/*`（如采用 child lineage 展示收口）
- 用户可见影响：
  - 子代理默认更少无意义 fan-out
  - `observe` 更像“为了下一步决策的查询”，而不是状态轮询
  - child branch 的完成、复用和关闭行为更可预测
- 开发者可见影响：
  - prompt 与 runtime 的职责边界更清晰
  - 四工具模型会形成正式 spec，而不是散落在描述文案里的软约定
  - 后续评估系统可以基于稳定协作契约度量工具价值

## Non-Goals

- 不引入 Claude Code 的 fork/self-fork 或 teammate network 运行模型。
- 不替换当前 `direct-parent + durable mailbox + wake` 的核心编排架构。
- 不在本 change 中建设 agent-tool 的评估/telemetry 系统；那属于独立的 `measure-agent-tool-effectiveness` change。
