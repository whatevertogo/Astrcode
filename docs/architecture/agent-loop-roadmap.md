# AgentLoop 升级路线图

> 最后更新：2026-04-05
> 范围：`crates/runtime-agent-loop/`、`crates/core/`、`server/` 及后续 Agent / Session 扩展

本文档基于以下输入整理：
- 现有实现：`crates/runtime-agent-loop/src/agent_loop.rs`、`crates/runtime-agent-loop/src/agent_loop/turn_runner.rs`、`crates/runtime-agent-loop/src/agent_loop/tool_cycle.rs`
- Agent 控制平面：`crates/runtime-agent-control/src/lib.rs`
- Agent 执行接线：`crates/runtime/src/service/agent_execution.rs`
- 对比分析：`docs/compare/agent-loopcompare.md`
- 现有设计稿：`docs/design/agent-tool-and-api-design.md`
- 参考仓库：Codex、Kimi-CLI、OpenCode、Pi-Mono、claude-code-sourcemap

目标不是推翻现有 `AgentLoop`，而是在保持当前分层边界成立的前提下，把 Astrcode 从“单 Agent 执行内核”升级为“可委派、可恢复、可集成、可控”的 Agent Runtime。

---

## 0. 已完成借鉴（已落地）

以下借鉴点已在项目内落地，后文不再作为“可借鉴项”重复讨论：

- 来自 Codex：控制平面与执行引擎分离（`runtime-agent-control` 独立承载 `AgentControl`）。
- 来自 Claude Code：按副作用/并发安全进行工具并发执行（P2 已完成）。
- 来自 OpenCode/Kimi：主 Agent 通过 `runAgent` 调起子 Agent，并返回结构化结果。
- 来自 OpenCode：Agent Profile 进入配置/加载体系（builtin + loader + hot reload）。
- 来自 Pi-Mono：子 Agent 事件带 parent-child 元数据并可回灌主会话。

---

## 1. 当前基线

已完成能力（仅保留核心设计与作用）：

| 阶段 | 状态 | 当前价值 |
|------|------|----------|
| P1 状态机化 | ✅ | 明确 turn 终态，统一收敛执行结果 |
| P2 并行工具执行 | ✅ | 按副作用分组并发，提升工具吞吐 |
| P3 压缩 + Token Budget | ✅ | 上下文构建分层，预算控制可插拔 |
| P4 错误恢复 | ✅ | 超长上下文与续跑异常可恢复 |
| P5 Agent Control Plane | 🟡 | 控制平面与执行引擎解耦，支持父子取消传播 |
| P6 runAgent MVP | 🟡 | 子 Agent 可调用、可裁剪、可回灌结果 |

当前内核已经完成“执行层分层 + 控制层外置”的基础形态，后续重点是把这些能力稳定暴露给 session/API/UI。

---

## 2. 外部实现的可借鉴点

### 2.1 Codex

仍可借鉴：
- `ToolOrchestrator` 统一承接审批、重试、网络策略，避免策略分散在 `tool_cycle.rs`。
- 子 Agent fork 时做最小上下文迁移，减少状态复制和污染。

不要直接照搬：
- Codex 的 Guardian + 平台沙箱很重，Astrcode 现在还没有稳定的 agent/session control plane，先做它会把复杂度放错地方。

### 2.2 Kimi-CLI

仍可借鉴：
- 根交互面统一审批，子 Agent 保持无交互执行，避免审批链路分叉。
- checkpoint/revert 与持久化上下文配套，给恢复/时间旅行提供真实锚点。
- 子任务结果继续强化为“摘要 + 工件”，避免主上下文污染。

不要直接照搬：
- `D-Mail` 是高级能力，前提是 checkpoint/revert、上下文持久化和 UI 语义都稳定。Astrcode 现在不应该把它排到最前。

### 2.3 OpenCode

仍可借鉴：
- 把当前 child turn 升级为独立 child session，建立完整生命周期与可观察性。
- 补齐 session 操作面：`prompt_async`、`abort`、`fork`、`revert`、`summarize`。
- 让 Profile 权限与 API 执行面完全同构，避免 prompt 层和运行时策略漂移。

不要直接照搬：
- OpenCode 的 API 面很宽，但它的状态和权限模型是围绕 session 数据库建的。Astrcode 应先补 session/turn 生命周期，再扩路由。

### 2.4 Pi-Mono

仍可借鉴：
- 事件协议继续标准化为稳定序列，降低 UI/IDE 集成成本。
- `before/after` hook 与 steer/follow-up 中途注入能力。
- 将 RPC 作为协议投影层，而不是新建第二套执行内核。

不要直接照搬：
- Pi 的抽象很轻，但也意味着很多约束留给上层。Astrcode 现阶段更适合保留强约束的 Rust core，再对外投影。

### 2.5 Claude Code / sourcemap

仍可借鉴：
- bridge/session runner 式控制面，把权限请求和会话活动状态统一外置。
- session memory 的“外部记忆 + 保留窗口”分离策略，支持长期对话质量稳定。

不要直接照搬：
- Claude 这套实现高度依赖其现有 bridge、remote session 和 UI 体系，Astrcode 先做本地单实例闭环更务实。

---

## 3. 对 Astrcode 的现实诊断

### 3.1 已有优势

- Loop 内核边界清晰，适合继续扩。
- `policy + approval + hook + compaction` 已经是可复用协作者。
- `tool_cycle.rs` 已经天然适合承接统一 tool orchestrator。

### 3.2 当前缺口

| 缺口 | 现状 | 影响 |
|------|------|------|
| Agent 控制平面 | 已有 `runtime-agent-control::AgentControl` 与 `SubAgentHandle` | 仍缺 child session 的完整操作面 |
| 子 Agent 生命周期 | 已有 parent-child 元数据与父取消传播 | 仍缺独立 child session 生命周期 |
| Agent Profile 真正接线 | loader + 内置 profile + 热重载已可运行 | 仍缺 profile 与 API 执行面的统一约束 |
| Session 操作面 | 已有异步提交、`interrupt`、`compact`、忙时自动分支 | 仍缺显式 `prompt_async`/`fork`/`revert`/turn status API |
| 中途控制 | 缺少 steer/follow-up 之类的 mid-turn 注入 | UI/IDE 难以实时干预执行 |
| 安全执行统一层 | 当前审批、策略、并发仍以 `tool_cycle.rs` 为主；子 Agent 侧已有 `SubAgentPolicyEngine` 收窄 | 后续接沙箱/网络策略时会变脆 |

### 3.3 一个重要判断

Astrcode 下一步最该做的不是：
- 先上 D-Mail
- 先上平台沙箱
- 先上大而全 HTTP API

最该做的是：
- 先把 Agent 从“单 turn 执行器”升级成“带父子关系的可控任务单元”
- 然后再把这套任务单元暴露给 API / UI / 插件

---

## 4. 升级原则

### 4.1 保持内核小，向外增加控制层

`AgentLoop` 应继续只负责：
- 构建请求
- 调用模型
- 执行工具
- 产出事件

不应直接承担：
- session 树管理
- 子 Agent 注册表
- 路由层协议
- 远程会话代理

### 4.2 子 Agent 先做成“受控 child session”，不要先做成“无限递归 loop”

更推荐的落地方向是：
- 主 Agent 发起 `runAgent` 工具调用
- runtime 创建 child session / child turn
- child session 运行自己的 `AgentLoop`
- 主 Agent 收到摘要化结果

这是 OpenCode/Kimi/Codex 的共同方向，只是名字不同。

### 4.3 审批只在根交互面出现

子 Agent 不应自行弹审批。

统一策略：
- 子 Agent 默认继承父策略的子集
- 子 Agent 触发需要审批的调用时，路由回根会话审批
- 如果当前运行场景不支持审批，则直接 deny，而不是阻塞在黑盒里

### 4.4 先补事件模型，再补 API

如果没有稳定的 parent-child event 关系、turn status 和取消语义，API 只会变成表面上可调用，实际上不可控。

---

## 5. 升级路线

### P5：Agent Control Plane（基础能力已落地，建议收口）

#### 目标

把 Astrcode 从“单 Agent turn runner”升级成“能管理多个 agent 实例的 runtime”。

#### 当前进展（2026-04-05）

1. `core` 已提供 `AgentProfile` / `AgentStatus` / `SubAgentHandle`
2. `runtime-agent-control` 已提供 spawn/list/cancel/wait 与父取消传播
3. `StorageEvent` 已通过 `AgentEventContext` 承载父子关系元数据
4. `runtime` 已接线控制平面（turn 结束/中断触发子树取消）

#### 剩余工作

1. 把 child 从“关联 turn”升级为“独立 child session 生命周期”
   - 支持独立创建、查询、取消、等待与回收
2. 增加控制平面可观测性
   - 活跃/终态分布、取消来源、失败类型等指标
3. 补齐最小查询面
   - 提供 agent/turn 状态查询接口，避免 UI 仅靠事件推断
4. 继续收紧边界
   - 保持 `runtime-agent-control` 不耦合 server/transport 细节

#### 推荐落点

- `crates/runtime-agent-loop/src/agent_loop.rs`
  - 保持执行器角色，不直接持有 registry
- `crates/runtime-agent-control/src/lib.rs`
   - 作为独立 crate 承载 `AgentControl` 控制平面
- `crates/core/`
  - 补 agent/profile/session tree 的核心 DTO 和 trait
- `crates/core/src/registry/`
  - 继续只放能力注册，不要把 agent registry 硬塞进去

#### 验收标准

- child session 生命周期可独立查询、取消、等待
- 父 turn 取消时，child agent 同步取消
- 事件流里能看出父子关系
- 不引入 server 依赖到 `runtime-agent-loop`

---

### P6：`runAgent` / `task` 工具 MVP（已可用，建议补齐隔离语义）

#### 目标

让主 Agent 可以通过工具调用子 Agent，但只做最小闭环，不一次做全功能多 agent 编排

#### 设计取向

这里更应该靠近 OpenCode/Kimi，而不是直接做 Codex 式任意多线程树：
- 每次工具调用创建一个 child session
- child session 使用独立上下文
- 返回结果必须摘要化，而不是把完整中间对话灌回主上下文

#### 交付物

1. `runAgent` 工具
   - 参数：`agent/profile`、`task`、`context`、`max_steps?`
2. child session 启动器
   - 复用现有 `AgentLoop`
   - 允许子 Agent 使用更小模型
3. 结果摘要器
   - 统一把 child session 结果折叠为 `summary + artifacts + task_id`
4. 权限收窄
   - 默认只读 profile：`explore` / `review`
   - 写 profile 单独开启：`execute`

#### 第一批内置 Profile

| Profile | 能力 | 默认权限 |
|---------|------|----------|
| `explore` | 检索、阅读、归纳 | 只读 |
| `plan` | 分析和拆解 | 只读，无编辑 |
| `review` | 审查和风险识别 | 只读 |
| `execute` | 定向修改 | 受限写权限 |

#### 当前进展（2026-04-05）

- 已支持 `runAgent` 调用子 Agent
- 结果已结构化返回（completed/failed/aborted/token_exceeded）
- 工具权限已按 profile 裁剪（allow/deny）
- 子事件已回灌主会话并带父子元数据

#### 剩余工作

- 当前实现仍以“同 session 的 child turn”为主，尚未升级为“独立 child session”
- 尚未实现递归深度上限（例如 2 或 3）与循环调用防护
- 需要把 child 结果的 `summary + artifacts + task_id` 契约进一步稳定化

#### 验收标准

- 主 Agent 能调用 `runAgent(explore, ...)`
- 子 Agent 输出不会污染主对话历史
- 子 Agent 失败/取消会以结构化 tool result 返回
- 禁止无限递归调用，默认深度上限 2 或 3

---

### P7：Session/Turn 操作面升级

#### 目标

补齐 OpenCode 已验证有效的 session control 面，而不是只保留“prompt 一次，SSE 看结果”。

#### 当前进展（2026-04-05）

- `submit_prompt` 已提供 202 异步执行语义
- 已提供会话级 `interrupt` 与 `compact`
- 已支持忙时自动分支（非显式 `fork` API）

#### 必做能力（剩余）

1. 标准化 `prompt_async` API
   - 统一 fire-and-forget 语义与返回体
2. `abort`
   - 补齐 turn 级取消与状态可见性
3. `fork`
   - 从指定 message/turn 派生新 session
4. `revert`
   - 回滚到指定 turn 或 message
5. turn status 查询
   - 明确 Pending/Running/Completed/Cancelled/Failed 对外读取接口

#### 为什么排在 P7

因为这一步依赖 P5/P6 收口：
- 目前 parent-child metadata 已有，但 child session 生命周期尚未独立
- 没有标准 turn status API，`async` 仍是半成品

#### 验收标准

- Server 能查询 turn status
- 子 Agent session 能被单独查看和取消
- `fork/revert` 不破坏当前 compaction 语义

---

### P8：中途控制与事件协议升级

#### 目标

引入类似 Pi-Mono 的“steering/follow-up/hooks/event contract”，让 Astrcode 更适合集成到 IDE、Tauri 和自动化入口。

#### 交付物

1. 事件协议标准化
   - `agent_start`
   - `turn_start`
   - `assistant_delta`
   - `tool_call_start`
   - `tool_call_end`
   - `turn_end`
   - `agent_end`
2. 中途注入能力
   - steering message：当前 tool 批次完成后立即插入
   - follow-up message：当前 agent stop 后插入
3. hook surface
   - `before_tool_call`
   - `after_tool_call`
   - `before_llm_request`
   - `after_turn`

#### 价值

- 对 Tauri/IDE 更友好
- 更适合做 RPC/SDK
- 后续 plugin 不需要侵入 loop 内部

#### 验收标准

- UI 能订阅稳定事件序列
- 在 agent 执行中可以注入 steer/follow-up
- hook 执行失败不会破坏核心状态机

---

### P9：上下文恢复与长期记忆

#### 目标

在现有 compaction 基础上补齐“可恢复、可回滚、可持久化理解”的能力。

#### 分阶段建议

##### P9.1 Checkpoint / Revert 基础设施

先做：
- turn checkpoint
- compact 前后 checkpoint
- child session checkpoint

##### P9.2 Session Memory

参考 Claude：
- 把“摘要”从单条 compact summary 升级成可单独存储的 session memory
- 对话窗口只保留最近高价值片段

##### P9.3 D-Mail / 时间旅行实验

最后再评估是否引入 Kimi 式 D-Mail。

理由很直接：
- 没有 checkpoint/revert，D-Mail 只是噱头
- 没有 UI 语义，用户很难理解“为什么 agent 改口了”

---

### P10：安全执行层

#### 目标

把当前 `tool_cycle.rs` 中的策略、审批、并发能力升级为真正的工具执行控制层。

#### 设计方向

参考 Codex / Claude：
- `ToolOrchestrator`
  - approval
  - sandbox selection
  - retry/escalation
  - network policy
- 工具并发与副作用分类继续复用现有 `concurrency_safe`

#### 现在不要做的事

- 不要一开始就追求跨平台完备沙箱
- 不要把所有平台差异压进 `AgentLoop`

先做：
- 统一 orchestrator 抽象
- 审批缓存
- 风险分类

再做：
- Linux/macOS/Windows 平台执行隔离

---

## 6. 推荐实现顺序

```text
P5/P6 收口（控制平面 + runAgent）
  -> P7 session/turn control API
     -> P8 steering + hook + event protocol
        -> P9 checkpoint/session memory
           -> P10 sandbox/orchestrator hardening
```

原因：
- 这是从“可运行”到“可集成”再到“可恢复/可控”的顺序。
- 反过来做会让大量 API 和安全设计没有稳定承载对象。

---

## 7. 明确不建议的路径

### 7.1 不建议先做“大而全开放 API”

没有 turn status、child session、abort 语义之前，API 只是把不稳定内部状态暴露出去。

### 7.2 不建议先做 D-Mail

它建立在 checkpoint/revert/持久化 message identity 之上，Astrcode 还没到这一步。

### 7.3 不建议让子 Agent 直接共享主上下文

这样会立刻带来：
- token 污染
- 审批语义混乱
- revert/fork 不可解释

### 7.4 不建议把 Agent registry 塞进 Capability registry

工具和 agent 都会被模型调用，但它们不是一类对象：
- tool 是 capability dispatch
- agent 是 session/turn lifecycle

这两个注册表应该并列，不应该互相伪装。

---

## 8. 第一批实际改动建议

如果按最小可落地补丁推进，建议第一批只做这些：

1. 把 `runAgent` 从“同 session child turn”升级为“独立 child session”
2. 增加 sub-agent 递归深度上限与循环调用防护
3. 给 server 增加 turn status 查询与 turn 级 `abort`
4. 增加显式 `fork/revert` API，并与 compaction 语义对齐
5. 把 `/api/v1/agents/{id}/execute` 与 `/api/v1/tools/{id}/execute` 从骨架升级为可执行路径

这样可以最快形成第一个真实闭环：
- 主 Agent
- 调用子 Agent
- 子 Agent 独立执行
- 事件可见
- 结果回主 Agent
- 可以取消

---

## 9. 与现有设计稿的关系

`docs/design/agent-tool-and-api-design.md` 的总体方向没有错，但当前最大问题是把几件事绑得太紧：
- Agent as Tool
- 开放 API
- Profile 配置
- 安全策略

升级路线应该改成分层推进：
- 先收口已落地的 AgentControl 和 `runAgent`
- 再把 child session 做实
- 再扩 session API
- 最后补高级记忆与安全隔离

这样才能避免在 `runtime-agent-loop` 还没稳定之前，把 server、plugin、UI 一起拖进改动面。

---

## 10. 结论

Astrcode 当前最有价值的资产不是“功能数量”，而是 `AgentLoop` 已经有比较好的执行内核边界。升级路线应该围绕这个优势展开：

- 不重写 loop
- 不先堆 API
- 不先追高级花活

先把 Agent 变成一等运行对象，再把它变成工具、会话和 API 的共同底座。
