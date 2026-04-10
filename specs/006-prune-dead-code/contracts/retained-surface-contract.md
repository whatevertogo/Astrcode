# Contract: 清理后保留的 Surface

本合同定义清理完成后，哪些 surface 仍属于当前正式支持面。
每个保留 surface 都标注了用途、消费者类型和所属边界（FR-014）。

## 1. Frontend

### 保留

| Surface | 用途 | 消费者 | 所属边界 |
|---------|------|--------|----------|
| 状态驱动导航 (`activeProjectId`, `activeSessionId`, `activeSubRunPath`) | 驱动会话列表、消息区、子线程面包屑的导航切换 | `useAgent` hook, `App` 组件 | `frontend/src/hooks/useAgent.ts` |
| focused subrun 浏览 | 在主会话内浏览子线程树的消息、嵌套 subRun 卡片 | `SubRunViewPanel`, `SubRunThreadItem` | `frontend/src/lib/subRunView.ts` |
| child session 直开 | 从 parent 通过 `childRef.openSessionId` 跳转到独立子会话 | `SubRunCard` 点击行为 | `frontend/src/lib/subRunView.ts` |
| 当前消息提交 | 用户发送消息并接收 SSE 增量更新 | `Composer` 组件 | `frontend/src/hooks/useAgent.ts` → `submitPrompt` API |
| 会话切换 | 切换活跃会话，触发历史加载与 SSE 订阅 | `SessionList` 组件 | `frontend/src/hooks/useAgent.ts` |
| SSE 连接 | 接收实时 agent 事件并分发到前端状态 | `useAgent` hook | `frontend/src/lib/applyAgentEvent.ts` |
| Settings 配置读写 | 读取/保存配置与模型选择 | `Settings` 组件 | `frontend/src/lib/api/` |
| `legacyDurable` 状态源标识 | 区分旧数据并渲染稳定错误而非降级视图 | `SubRunStatusSource` 类型 | `frontend/src/types.ts` |
| `hasDescriptorLineage` 字段 | 区分 parentTurnId-based 新 lineage 与栈推导旧数据 | `SubRunRecord`, `SubRunViewData` | `frontend/src/lib/subRunView.ts` |
| `ChildSessionNotificationKind` (含 `waiting`) | 兼容 durable child 通知读侧旧事件样本 | `childSessionNotification` 消息 | `frontend/src/types.ts` |

### 不保留

- 无消费者的 parent-child summary projection (`buildParentSummaryProjection`)
- 仅为 legacy downgrade 展示保留的 `ChildSessionViewResponseDto` 等类型和 helper
- duplicated child open flag (`openable`)
- 前端 session summary/view client (`loadParentChildSummaryList`, `loadChildSessionView`)
- 旧 cancel route client wrapper

## 2. Server HTTP

### 保留

| Surface | 用途 | 消费者 | 所属边界 |
|---------|------|--------|----------|
| `GET /api/sessions` | 列出当前项目的所有会话 | `SessionList` 组件 | `crates/server/src/http/routes/sessions/query.rs` |
| `POST /api/sessions` | 创建新会话 | `App` 组件 | `crates/server/src/http/routes/sessions/mutation.rs` |
| `GET /api/sessions/{id}/history` | 获取会话历史消息（含子线程） | `MessageList` 组件 | `crates/server/src/http/routes/sessions/query.rs` |
| `GET /api/sessions/{id}/events` | SSE 实时事件流 | `useAgent` hook | `crates/server/src/http/routes/sessions/query.rs` |
| `POST /api/sessions/{id}/prompts` | 提交用户消息 | `Composer` 组件 | `crates/server/src/http/routes/sessions/mutation.rs` |
| `POST /api/sessions/{id}/compact` | 手动触发上下文压缩 | `useAgent` hook | `crates/server/src/http/routes/sessions/mutation.rs` |
| `POST /api/sessions/{id}/interrupt` | 中断当前正在进行的 turn | `useAgent` hook | `crates/server/src/http/routes/sessions/mutation.rs` |
| `DELETE /api/sessions/{id}` | 删除单个会话 | `SessionList` 组件 | `crates/server/src/http/routes/sessions/mutation.rs` |
| `DELETE /api/projects` | 删除整个项目 | `Settings` 组件 | `crates/server/src/http/routes/sessions/mutation.rs` |
| `GET /api/config` | 获取当前配置视图 | `Settings` 组件 | `crates/server/src/http/routes/config.rs` |
| `POST /api/config/reload` | 从磁盘重新加载配置并热替换 runtime loop | CLI / 开发工具 | `crates/server/src/http/routes/config.rs` |
| `POST /api/config/active-selection` | 保存活跃的 profile/model 选择 | `Settings` 组件 | `crates/server/src/http/routes/config.rs` |
| `GET /api/models/current` | 获取当前激活的模型信息 | `Settings` 组件 | `crates/server/src/http/routes/model.rs` |
| `GET /api/models` | 列出所有可用模型选项 | `Settings` 组件 | `crates/server/src/http/routes/model.rs` |
| `POST /api/models/test` | 测试模型连接 | `Settings` 组件 | `crates/server/src/http/routes/model.rs` |
| `GET /api/v1/agents` | 列出可用 Agent Profiles | 前端 Agent 选择 UI | `crates/server/src/http/routes/agents.rs` |
| `POST /api/v1/agents/{id}/execute` | 创建 root execution 并返回 session/turn 标识 | 前端 root execution 入口 | `crates/server/src/http/routes/agents.rs` |
| `GET /api/v1/sessions/{id}/subruns/{sub_run_id}` | 查询子会话执行状态 | 前端 subrun 轮询 | `crates/server/src/http/routes/agents.rs` |
| `POST /api/v1/sessions/{id}/agents/{agent_id}/close` | 关闭 agent 及其子树 | 前端 closeAgent 按钮 | `crates/server/src/http/routes/agents.rs` |
| `GET /api/session-events` | SSE 会话目录变更事件 | 前端会话列表实时更新 | `crates/server/src/http/routes/sessions/query.rs` |
| `GET /api/sessions/{id}/composer/options` | 获取会话级 composer 配置 | `Composer` 组件 | `crates/server/src/http/routes/composer.rs` |

### 不保留

| 已删除 Surface | 原用途 | 删除原因 |
|----------------|--------|----------|
| `/api/sessions/{id}/children/summary` | 返回 parent-child summary 列表 | 无前端消费者 |
| `/api/sessions/{id}/children/{child_session_id}/view` | 返回 child session 详细视图 | 无前端消费者 |
| `/api/runtime/plugins` | 运行时插件管理 | skeleton route，无实际消费者 |
| `/api/v1/tools` | 工具列表/执行 | skeleton route，无实际消费者 |

## 3. Runtime / Core / Protocol

### 保留

| Surface | 用途 | 消费者 | 所属边界 |
|---------|------|--------|----------|
| `AgentStatus` 枚举 | subrun/child 唯一状态模型 | core → protocol → frontend 三层 | `crates/core/src/agent/mod.rs` |
| `ExecutionAccepted` | 唯一内部 execution receipt | runtime service contract | `crates/core/src/runtime/traits.rs` |
| `SubRunHandle` | 唯一 lineage owner | runtime execution layer | `crates/core/src/runtime/traits.rs` |
| `child_ref.open_session_id` (protocol DTO) | 唯一 child open target | protocol → server mapper → frontend | `crates/protocol/src/http/agent.rs` |
| `PromptMetricsPayload` | 唯一共享指标字段集合 | core event → protocol DTO → frontend | `crates/core/src/event/types.rs` |
| `closeAgent` 主线 | 唯一 child close/cancel 操作 | server route → runtime control | `crates/runtime/src/service/execution/subagent.rs` |
| `ChildAgentRef` (不含 `openable`) | canonical child 引用，传递 agent/session/execution 标识 | core event → durable/session write | `crates/core/src/event/types.rs` |
| `StorageEventPayload::PromptMetrics` | 持久化层 prompt metrics 事件 | storage event log | `crates/core/src/event/types.rs` |
| `AgentEvent::PromptMetrics` | 前端事件层 prompt metrics | SSE 推送 | `crates/core/src/event/domain.rs` |
| `CompactionTrigger` (含 `Reactive` 内部语义) | 压缩触发原因映射 | agent loop → core hook | `crates/core/src/hook.rs` |

### 不保留

| 已删除 Surface | 原用途 | 替代 |
|----------------|--------|------|
| `SubRunOutcome` | 旧 subrun 完成状态枚举 | `AgentStatus` |
| `SubRunDescriptor` | 旧 lineage 元数据容器 | `parentTurnId` 直接在 SubRunHandle 上 |
| `PromptAccepted` / `RootExecutionAccepted` | 重复 execution receipt | `ExecutionAccepted` |
| protocol `status: String` | 无类型约束的状态字段 | 强类型 `AgentStatus` 枚举 |
| notification 外层 `open_session_id` | 重复的 open target | `child_ref.open_session_id` 唯一入口 |
| `ChildAgentRef.openable` | 重复的 open flag | `open_session_id` 非空即 openable |
| `descriptorless` / `legacyDurable` downgrade source | 旧数据降级视图 | 明确失败路径 |

## 4. Enforcement

- 任一保留 surface 都必须能指向当前真实消费者与 owner boundary（见上表）。
- 任何新增 surface 若没有 owner 和消费者，不得以"以后也许会用"为理由进入主线。
