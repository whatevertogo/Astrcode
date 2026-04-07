# Agent as Tool / 子代理系统规范

## 1. 范围与状态

本文档定义：

- `spawnAgent` 的公开工具面
- 子执行的核心数据模型
- 生命周期事件语义
- 存储模式、控制面、策略与 API 约束

当前主线状态：

- `spawnAgent + controlled sub-session` 为正式主线
- `SharedSession` 为正式路径
- `IndependentSession` 为 experimental 扩展面

## 2. 术语

| 术语 | 含义 |
| --- | --- |
| Agent Profile | 子 Agent 的画像定义，决定模式、工具边界和预算偏好 |
| SubRun | 一次子执行实例 |
| SharedSession | 子执行事件写入父 session |
| IndependentSession | 子执行事件写入独立 child session |
| Handoff | 子执行完成后交给父流程或 UI 的结构化结果 |

## 3. 规范性约束

### 3.1 必须保持的设计结论

- `spawnAgent` **MUST** 保持极简公开 schema。
- 子执行生命周期 **MUST** 通过 `SubRunStarted / SubRunFinished` 暴露。
- 父流程消费结果时 **MUST** 优先使用 `SubRunFinished.result`。
- session 写入模式 **MUST NOT** 决定任务 ownership、kill 或 cleanup 责任。
- 子 Agent **MUST NOT** 直接共享父可变运行时状态。

### 3.2 当前不采纳的路线

- 不公开 `isolated_session` 之类的自由 session override
- 不新增 `ChildSessionSummary` 平行摘要事件
- 不在 `spawnAgent` 中恢复 `tasks[]` / DAG 编排语义
- 不把 UI 状态或权限提示直通给子 Agent

## 4. 公开数据模型

### 4.1 `AgentProfile`

| 字段 | 说明 |
| --- | --- |
| `id` | 唯一标识 |
| `name` | 显示名称 |
| `description` | 用途描述 |
| `mode` | `Primary` / `SubAgent` / `All` |
| `system_prompt` | 子 Agent 系统提示 |
| `allowed_tools` | 白名单 |
| `disallowed_tools` | 黑名单 |
| `max_steps` | 步数上限 |
| `token_budget` | token 预算 |
| `model_preference` | 预留字段；当前未形成稳定生效链路 |

### 4.2 `SpawnAgentParams`

公开工具参数只包含：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `type` | 否 | profile 标识；为空时走默认 profile |
| `description` | 是 | UI / 日志摘要 |
| `prompt` | 是 | 子 Agent 实际任务正文 |
| `context` | 否 | 调用侧补充上下文 |

### 4.3 `SubagentContextOverrides`

这组字段只属于 root execution API 与内部执行装配，**不是** `spawnAgent` 的公开 schema。

当前约束：

- `storage_mode` 可在内部选择 `SharedSession` / `IndependentSession`
- `inherit_cancel_token = false` 当前不支持
- `include_recovery_refs = true` 当前不支持
- `include_parent_findings = true` 当前不支持

### 4.4 `SubRunResult`

`SubRunResult` 包含：

| 字段 | 说明 |
| --- | --- |
| `status` | `Running` / `Completed` / `Failed` / `Aborted` / `TokenExceeded` |
| `handoff` | 完成时的结构化结果 |
| `failure` | 失败信息 |

`handoff` 至少承载：

- `summary`
- `findings`
- `artifacts`

`artifacts` 中可出现：

- `subRun` 引用
- `session` 引用（只在 `IndependentSession` 下可出现）

## 5. 执行协议

### 5.1 启动与返回

`spawnAgent` 被调用后：

1. runtime 解析 `SpawnAgentParams`
2. 根据 profile 与上下文装配子执行
3. 启动子执行
4. 立即返回一个普通 tool result

如果子执行仍在后台运行，tool result 可返回 `Running` 状态与结构化句柄；后续真实进展必须通过生命周期事件观察。

### 5.2 生命周期事件

父侧统一消费：

- `SubRunStarted`
- `SubRunFinished`

`SubRunFinished` 必须承载：

- `result`
- `step_count`
- `estimated_tokens`
- `child_session_id`（如果存在）

### 5.3 tool call 与 subrun 的关联

协议 **MUST** 保证 `spawnAgent` tool call 与 subrun 生命周期之间存在稳定关联。

当前允许两种实现：

1. **首选**：`SubRunStarted / SubRunFinished` 显式携带 `tool_call_id`
2. **兼容**：明确规定“同一 `turn_id` 内按发出顺序 1:1 配对”

长期推荐方案仍然是补 `tool_call_id`。

## 6. 存储模式与控制面

### 6.1 `SharedSession`

- 子执行事件写入父 session
- 是当前正式主线
- 更利于父流程直接消费子执行结果

### 6.2 `IndependentSession`

- 子执行事件写入独立 child session
- 允许通过 `child_session_id` 建立导航
- 当前仍为 experimental

### 6.3 root-owned task control

以下能力 **MUST** 归根执行域统一管理：

- shell / MCP / 长任务注册
- kill / cleanup / timeout
- 取消级联
- 任务观测与回收责任

session mode 只定义事件落盘位置，不承载控制平面语义。

## 7. 策略与安全

### 7.1 工具边界

子 Agent 的最终可用工具集由下面几层共同决定：

1. 父策略上界
2. profile 的 `allowed_tools`
3. profile 的 `disallowed_tools`
4. 执行装配期附加限制

`disallowed_tools` 优先级高于 `allowed_tools`。

### 7.2 审批与交互

当前默认约束：

- 子 Agent 通常没有独立 UI 审批能力
- 需要审批的调用默认不应直接弹子交互面
- 若未来支持审批回根会话，必须通过统一控制面与事件协议实现

### 7.3 取消传播

- 父取消 **MUST** 级联子取消
- 当前不支持取消链路被子执行中断

## 8. API 面

### 8.1 已稳定可用

| 路由 | 说明 |
| --- | --- |
| `GET /api/v1/agents` | 列出可用 Agent Profile |
| `POST /api/v1/agents/{id}/execute` | 创建 root execution |
| `GET /api/v1/sessions/{id}/subruns/{sub_run_id}` | 查询子执行状态 |
| `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel` | 显式取消后台子执行 |

### 8.2 当前未收口

| 路由 | 状态 |
| --- | --- |
| `POST /api/v1/tools/{id}/execute` | 仍为 501 骨架，不是正式执行入口 |

## 9. Profile 加载

加载优先级：

1. `builtin://`
2. `~/.claude/agents/`
3. `~/.astrcode/agents/`
4. `<working_dir>/.claude/agents/`
5. `<working_dir>/.astrcode/agents/`

Profile 文件负责提供：

- 名称与描述
- 子 Agent 提示词
- 工具白黑名单
- 预算与偏好

## 10. 非目标

- 不在当前阶段定义多子任务 DAG 编排
- 不在当前阶段让子 Agent 修改父 AppState
- 不在当前阶段把 `IndependentSession` 升格为默认路径

## 11. 对应文档

- 设计入口：[../design/agent-tool-and-api-design.md](../design/agent-tool-and-api-design.md)
- 开放项：[./open-items.md](./open-items.md)
