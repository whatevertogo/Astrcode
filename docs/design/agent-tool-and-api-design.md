# Agent as Tool（子代理系统）设计

## 问题

Astrcode 需要让主 Agent 把工作委派给子 Agent，但不能把子 Agent 做成“另一个不受控的主会话”。

设计的关键不是“能不能起子 Agent”，而是：

- 子执行如何被限制
- 子执行结果如何回流
- session / tool call / 生命周期之间如何稳定关联

## 目标

1. 让 `spawnAgent` 成为稳定、简单、可长期演进的工具入口。
2. 让子执行拥有独立的执行边界，但不破坏父会话真相。
3. 让 UI 和父流程都能通过统一事件消费子执行结果。

## 主线方案

### 1. 受控子会话，而不是自由分叉

主线模型固定为：

- `spawnAgent + controlled sub-session`
- `SharedSession` 为正式路径
- `IndependentSession` 仍为 experimental 扩展面

### 2. `spawnAgent` 保持极简 schema

公开工具参数只保留：

- `type`
- `description`
- `prompt`
- `context`

`storage_mode`、继承控制、执行上界等能力留在内部执行装配或 root execution API，不直接暴露给 LLM。

### 3. 生命周期事件是父侧真相

父流程和 UI 识别子执行，优先依赖：

- `SubRunStarted`
- `SubRunFinished`
- `SubRunFinished.result`

而不是设计一套平行摘要事件。

### 4. 结果以 handoff 为中心

`spawnAgent` 可以先返回 running 句柄，但真正稳定的消费面是 `SubRunFinished.result` 中的：

- `summary`
- `findings`
- `artifacts`
- `failure`

### 5. session 归属与任务归属分离

`SharedSession` / `IndependentSession` 只回答“事件写到哪里”。

shell、MCP、长任务、kill / cleanup / timeout 等控制责任，应该收口到 root-owned task control，而不是继续挂在 session mode 上。

## 明确边界

### 当前不做

- 不把 `isolated_session` 暴露成公开工具参数
- 不新增 `ChildSessionSummary` 一类平行结果事件
- 不让子 Agent 共享父可变状态
- 不在单个工具里恢复 DAG / `tasks[]` 编排语义

### 当前必须补强

- `spawnAgent` tool call 与 subrun 生命周期的稳定关联
- root-owned task control
- shared observability 的聚合能力

## 设计原则

1. 先固定协议真相，再讨论体验增强。
2. 先做 shared observability，不做 shared mutable state。
3. 不把控制面能力继续堆进 `SubagentContextOverrides`。

## 对应规范

- [../spec/agent-tool-and-api-spec.md](../spec/agent-tool-and-api-spec.md)
- [../spec/open-items.md](../spec/open-items.md)
