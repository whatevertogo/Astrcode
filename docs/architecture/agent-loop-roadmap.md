# Agent Runtime 路线图

> 最后整理：2026-04-07

## 1. 当前基线

Astrcode 已经具备 Agent Runtime 的核心骨架：

- `runtime-agent-loop`：执行循环
- `runtime-agent-control`：控制平面
- `spawnAgent`：子 Agent 工具入口
- `runtime-session`：session / turn 真相
- `runtime`：统一服务门面

所以路线图的重点不再是“能不能跑起来”，而是“如何把现有能力收口成稳定协议和产品面”。

## 2. 当前阶段最重要的判断

优先级应该是：

1. 先收口协议与控制面
2. 再补 session / subrun 的读模型与前端体验
3. 再扩 Agent 协作、长期记忆和生态集成

不应该先做：

- 大而全开放 API
- 没有控制面约束的多 Agent 自由编排
- 依赖复杂前置能力的 D-Mail / 时间旅行式实验
- 过重的安全执行平台化改造

## 3. 近期路线

### 3.1 P1：控制面与子执行协议收口

目标：让子 Agent 成为稳定、可观测、可取消、可关联的任务单元。

重点：

- root-owned task control
- subrun 与 tool call 的稳定关联
- 子执行结果与 observability 收口
- `IndependentSession` 继续保持 experimental

### 3.2 P2：session / subrun 读模型补齐

目标：让 UI 和 API 能稳定消费多会话、多 subrun 结构。

重点：

- durable `list_subruns(session_id)`
- `scope(self|subtree|directChildren)` 过滤
- child session 导航能力补齐

### 3.3 P3：Agent 协作能力增强

目标：补齐真正的多 Agent 协作面，而不是只靠一次性 handoff。

候选能力：

- agent 间消息传递
- 审批回根会话
- 更强的 shared observability aggregation

## 4. 中期路线

### 4.1 上下文恢复与长期记忆

在 compact、checkpoint 和 replay 边界稳定后，再推进：

- session memory
- 更精细的恢复与回退
- 更智能的上下文保留策略

### 4.2 扩展接口面

在核心协议稳定后，再考虑：

- ACP
- 更强的插件集成
- 更宽的外部 API 面

## 5. 长期路线

### 5.1 安全执行层

安全执行层仍重要，但应该建立在：

- 稳定的 policy 面
- 稳定的 task ownership
- 稳定的审批与控制通道

之上，而不是抢在控制面之前重做执行内核。

## 6. 路线约束

推进路线图时必须保持：

- `AgentLoop` 继续只做执行循环，不膨胀成总控中心
- session truth 与 subrun truth 继续由统一事件协议表达
- 子 Agent 默认是受控子会话，不是自由线程池
- 先做 shared observability，不做 shared mutable state

## 7. 对应清单

具体待办已经收口到：

- [../spec/open-items.md](../spec/open-items.md)
- [../spec/agent-tool-and-api-spec.md](../spec/agent-tool-and-api-spec.md)
- [../spec/session-and-subrun-spec.md](../spec/session-and-subrun-spec.md)
