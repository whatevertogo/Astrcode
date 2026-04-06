# Agent as Tool + 开放 API 实施计划

> **最后更新**：2026-04-05
> **当前状态**：Phase 0-2 已实现，Phase 3 已具备真实 root execution 与 sub-run 状态查询

```text
Phase 0 (设计完成) → Phase 1 (Agent Loader) → Phase 2 (Agent as Tool) ✅
    → Phase 3 (扩展 API) 🟡 进行中 → Phase 4 (WebSocket) → Phase 5 (前端适配 ✅ 已完成)
```

---

## Phase 0 — Phase 1 总结

### Phase 0: 设计 ✅ 已完成  
### Phase 1: Agent Loader ✅ 已完成  

- 已实现 Markdown 目录加载 Agent 定义
- builtin profile: explore / plan / review / execute
- 集成到 Runtime Bootstrap
- 支持热重载（`start_agent_auto_reload`）


## Phase 2: Agent as Tool 实现 ✅ 已完成

**实际实现**：

### 2.1 工具定义与参数（`crates/runtime-agent-tool/src/lib.rs`）
- `RunAgentTool` 通过 `SubAgentExecutor` trait 委托执行，不直接耦合 runtime
- `RunAgentParams` 序列化字段为 camelCase: `name`, `task`, `context`, `maxSteps`
- 参数验证失败时返回 `ok=false` 的 `ToolExecutionResult`，error 描述原因

### 2.2 子 Agent 执行服务（`crates/runtime/src/service/agent_execution.rs`）
- `AgentExecutionServiceHandle` 持有 `RuntimeService` 引用
- `DeferredSubAgentExecutor` 在 bootstrap 阶段占位，service 创建后 bind
- `execute_subagent` 执行路径：
  - 校验 `turn_id` 和 `event_sink`（来自 ToolContext）
  - 查找 profile → 校验 `AgentMode::SubAgent` → `resolve_profile_tool_names()` 按 allow/deny 裁剪可见工具
  - `runtime.agent_control.spawn()` 注册子 Agent，`mark_running()` → 获取 CancelToken
  - 构建 `SubAgentPolicyEngine`（禁止 Ask，只允许白名单工具）
  - `ChildExecutionTracker` 跟踪 step/token 预算，超限时 cancel
  - `event_sink.emit()` 写入子 Agent 的 `UserMessage` 和后续事件，带 `AgentEventContext`
  - 结果折叠为 `SubAgentResult(outcome, summary, metadata)` 回主 turn

### 2.3 策略与预算（`crates/runtime-agent-loop/src/subagent.rs`）
- `SubAgentPolicyEngine`: 包装父 PolicyEngine，白名单过滤 + 将 Ask 转为 Deny
- `ChildExecutionTracker`: 通过 `observe()` 监听事件流，步数/预算超限即 cancel
- 注意：`step_index >= max_steps` 是"软限制"（允许第 N+1 步部分执行后即取消）

### 2.4 路由注册（`crates/runtime/src/builtin_capabilities.rs`）
- 在 `built_in_capability_invokers()` 中注册 `RunAgentTool`，通过 `ToolCapabilityInvoker` 包装
- 所有内置工具统一走同一套 capability dispatch

### 2.5 事件投影（前端）
- `StorageEvent` 已通过 `AgentEventContext` 承载 `agent_id` / `parent_turn_id` / `agent_profile`
- 前端 `MessageList.tsx` 实现 `agentGroup` 嵌套 UI
- `applyAgentEvent.ts` 提取 agent 字段注入消息 action

### 2.6 API 端点（`crates/server/src/routes/`）
- `/api/v1/agents` — 列出 Agent Profiles（GET）
- `/api/v1/agents/{id}/execute` — 创建 root execution 并返回 `sessionId/turnId/agentId`
- `/api/v1/sessions/{id}/subruns/{sub_run_id}` — 查询子会话状态
- `/api/v1/tools` — 列出当前工具列表（GET）
- `/api/v1/tools/{id}/execute` — 返回 501 Not Implemented（骨架）

**验收标准**：
- ✅ `runAgent` 工具可被 LLM 调用
- ✅ 子 Agent 事件带父子元数据写入 JSONL
- ✅ 子 Agent 失败/取消返回结构化 tool result
- ✅ 前端可渲染子 Agent 消息分组

---

## Phase 3: 扩展 API 🟡 进行中

**目标**: 基于现有 server crate 扩展 API 端点，不引入独立 API crate

### 已实现

**文件**: `crates/server/src/routes/`

- `GET /api/v1/agents` → `routes/agents.rs`: 列出 Agent Profiles
  - 使用 `AgentExecutionServiceHandle::list_profiles()` → `AgentProfileDto`
- `POST /api/v1/agents/{id}/execute` → 创建独立 session 并异步启动 root execution
  - 返回 `accepted/sessionId/turnId/agentId`
- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}` → 查询子会话当前状态
  - 返回 `SubRunStatusDto`
- `GET /api/v1/tools` → `routes/tools.rs`: 列出当前运行时工具列表
  - 使用 `ToolExecutionServiceHandle::list_tools()` → `ToolDescriptorDto`
- `POST /api/v1/tools/{id}/execute` → 返回 `501 Not Implemented`
  - 提示当前应使用 session turn 或 `runAgent`

### 当前 override / 观测状态

- `runAgent.contextOverrides` 继续走有限 override：
  - 已支持：`storageMode`、`includeCompactSummary`、`includeRecentTail`
  - 显式拒绝：`inheritCancelToken=false`、`includeRecoveryRefs=true`、`includeParentFindings=true`
  - `inheritSystemInstructions` 与 `inheritProjectInstructions` 当前必须解析为相同值
- runtime status 已可暴露 `metrics.subrunExecution`
  - 用于聚合 sub-run 的 outcome / storage mode / step / estimated token
  - 不引入父状态直写或共享 callback

**DTO 定义**: `crates/protocol/src/http/agent.rs` + `crates/protocol/src/http/tool.rs`
- `AgentProfileDto`, `AgentExecuteRequestDto`, `AgentExecuteResponseDto`
- `ToolDescriptorDto`, `ToolExecuteRequestDto`, `ToolExecuteResponseDto`

**Router 注册**: `crates/server/src/routes/mod.rs` → `build_api_router()`

### 待完成

- `POST /api/v1/sessions/{id}/abort` — turn 级取消
- `POST /api/v1/sessions/{id}/fork` — 从指定 turn 派生新 session
- `POST /api/v1/sessions/{id}/revert` — 回滚到指定 turn
- `/api/v1/tools/{id}/execute` 从骨架升级为真实执行端点

### 多 Agent 后续 Roadmap

当前多 agent 语义已经从早期的“共享 session + 事件打标”演进到受控子会话，但后续扩展顺序需要继续保持克制：

1. `root-owned task registry`
   - 子 agent 启动的 shell / MCP / 长任务后续应统一归 runtime 根级 task registry 管理。
   - 目标是让 kill / cleanup / timeout 不再依赖 `shared_session` 或 `independent_session` 的存储语义。

2. `shared observability aggregation`
   - 允许父流程聚合子执行域的 step / token / outcome / findings / artifacts 摘要。
   - 不允许因此引入父状态直写能力；父侧消费继续基于 `SubRunFinished.result` 和生命周期事件。

3. `independent_session` 保持 experimental
   - 当前只保证可查询、可展示、可回填结果。
   - 在 root-owned task control 与 shared observability 清晰前，不扩大其产品承诺范围，不引入新的自由共享开关。

---

## Phase 4: WebSocket 实时通信

**目标**: 实现 WebSocket 双向通信, 支持实时交互

**预估时间**: 1 天

### 备注

当前已通过 SSE 事件流（`/api/sessions/:id/events`）+ 断点续传机制实现实时事件推送。
WebSocket 是备选方案，目前优先级较低。如后续需要真正的双向通信（如客户端主动下发 steer/follow-up），再评估引入。

（原有设计方案保留，但暂不实施）

---

## Phase 5: 前端适配  已较为简洁的完成

TODO: 更好的前端

### 已实现

**事件层**（`frontend/src/lib/applyAgentEvent.ts`）:
- 从 SSE 事件提取 `agentId` / `parentTurnId` / `agentProfile` 字段
- 通过 spread `...agentFields` 注入到所有消息 action（UserMessage, AssistantMessage, ToolCall, Compact）

**状态层**（`frontend/src/store/reducer.ts`）:
- 所有消息类型新增 `agentId`, `parentTurnId`, `agentProfile` 字段
- 所有 action 类型扩展对应字段声明

**类型层**（`frontend/src/types.ts`）:
- `UserMessage`, `AssistantMessage`, `ToolCallMessage`, `CompactMessage` 均新增可选 agent 字段
- Action 类型扩展对应属性

**渲染层**（`frontend/src/components/Chat/MessageList.tsx`）:
- `isNestedAgentMessage()` 检测带 `agentId + parentTurnId` 的消息
- 连续子 Agent 消息渲染为 `agentGroup`（header 显示 "子 Agent" + profile ID）
- 使用 `groupMessageRow` 类名区分嵌套消息样式
- CSS: `MessageList.module.css` 定义 `agentGroup` / `agentGroupHeader` / `agentGroupLabel` / `agentGroupTitle` / `agentGroupBody`

**验收标准**：
- ✅ 前端正确消费带 agent 元数据的 SSE 事件
- ✅ 状态机正确写入 agent 字段
- ✅ UI 将子 Agent 消息渲染为嵌套分组

---

## Phase 6: 测试与验证

### 已实现

- **Agent Loader 测试**: `crates/runtime-agent-loader/src/lib.rs` 内建多项测试（profile 加载、merge、Markdown/YAML 解析）
- **Agent Tool 测试**: `crates/runtime-agent-tool/src/lib.rs` 覆盖 params 解析 + 无效参数报错
- **RunAgent 集成**: `crates/runtime/src/service/agent_execution.rs` 中 `run_agent_tool_emits_child_events_with_agent_context` 端到端测试
- **API 路由测试**: `crates/server/src/runtime_routes_tests.rs` 覆盖 `/api/v1/agents`、`/api/v1/tools`、execute 端点 501
- **Agent Control 测试**: `crates/runtime-agent-control/src/lib.rs` 覆盖 spawn/list/cancel/wait/级联取消/GC

### 待补充

- [ ] `SubAgentPolicyEngine::check_capability_call` 三个分支测试（allow/deny/ask→deny）
- [ ] `CapabilityRouter::subset_for_tools` 测试
```

### 6.3 API 测试

使用 `curl` 验证所有端点:

```bash
# 健康检查
curl http://localhost:6543/health

# 列出 Agent
curl http://localhost:6543/agents

# 发送消息 (SSE)
curl -N http://localhost:6543/sessions/session-123/message \
  -H "Content-Type: application/json" \
  -d '{"content": "分析 src/ 目录下的代码结构"}'

# 执行 Agent 任务 (SSE)
curl -N http://localhost:6543/agents/explore/execute \
  -H "Content-Type: application/json" \
  -d '{"task": "查找所有使用 X 的地方", "working_dir": "/path/to/project"}'
```

---

## 总工作量评估

| Phase | 内容 | 预估时间 |
|-------|------|----------|
| Phase 0 | 基础设施 | 0.5 天 |
| Phase 1 | Agent Profile 系统 | 1 天 |
| Phase 2 | Agent as Tool | 2 天 |
| Phase 3 | 扩展 REST API | 2 天 |
| Phase 4 | WebSocket | 1 天 |
| Phase 5 | 前端集成 | 1 天 |
| Phase 6 | 测试验证 | 1 天 |
| **总计** | | **8.5 天** |

---

## 风险评估与缓解

| 风险 | 影响 | 概率 | 缓解措施 |
|------|------|------|----------|
| AgentLoop 重构影响现有功能 | 高 | 中 | 子 Agent 使用独立代码路径, 不修改现有 turn_runner |
| Token 预算控制失效 | 高 | 低 | 在 SubAgentExecutor 中强制检查 |
| SSE 流泄漏 (连接断开) | 中 | 中 | 使用 mpsc 的 `try_send`, 断开时自动清理 |
| 策略引擎绕过 | 高 | 低 | SubAgentPolicyEngine 在 tool_cycle 前拦截 |
| API Key 管理不善 | 高 | 低 | 通过环境变量, 不硬编码 |
| WebSocket 并发冲突 | 中 | 低 | Axum 的 ws 实现已处理并发 |

---

## 后续扩展 (Phase 7+)

- [ ] D-Mail 时间旅行 (参考 Kimi-CLI)
- [ ] 安全沙箱 (参考 Codex)
- [ ] Auto-configure Agent (LLM 自行创建子 Agent Profile)
- [ ] 多工作空间路由
- [ ] MCP Server 集成
- [ ] 分布式 Agent 编排

---

## 验证命令速查

```bash
# 全量检查
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode

# 单个 package
cargo test --package astrcode-runtime-agent-loader
cargo test --package runtime-agent-tool

# 运行 API 服务
cargo run --package runtime-agent-api

# 前端检查
cd frontend && npm run typecheck && npm run lint
```
