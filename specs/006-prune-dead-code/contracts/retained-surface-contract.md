# Contract: 清理后保留的 Surface

本合同定义清理完成后，哪些 surface 仍属于当前正式支持面。

## 1. Frontend

### 保留

- 状态驱动导航：
  - `activeProjectId`
  - `activeSessionId`
  - `activeSubRunPath`
- focused subrun 浏览与 child session 直开
- 当前消息提交、会话切换、SSE 连接
- 当前 Settings 所需的配置读写与模型选择能力

### 不保留

- 无消费者的 parent-child summary projection
- 仅为 legacy downgrade 展示保留的类型和 helper
- duplicated child open flag（如 `openable`）

## 2. Server HTTP

### 保留

- 会话历史与事件流主线
- 当前消息提交、中断、compact、删除
- 当前配置读取与保存活跃选择
- 当前模型读取、枚举与连通性测试

### 不保留

- `/api/sessions/{id}/children/summary`
- `/api/sessions/{id}/children/{child_session_id}/view`
- `/api/v1/agents*`
- `/api/v1/tools*`
- `/api/runtime/plugins*`
- `/api/config/reload`
- legacy cancel route（迁移完成后）

## 3. Runtime / Core / Protocol

### 保留

- 当前会话与子会话历史主线
- 当前 handoff summary / child notification summary
- `AgentStatus` 作为唯一 subrun 状态模型
- protocol 强类型状态枚举作为唯一 child/subrun 状态 DTO
- `ExecutionAccepted` 作为唯一内部 execution receipt
- `SubRunHandle` 作为唯一 lineage owner
- `child_ref.open_session_id` 作为唯一 child open target
- `PromptMetricsPayload` 作为唯一共享指标字段集合
- `closeAgent` 作为唯一 child close/cancel 主线

### 不保留

- `SubRunOutcome`
- `SubRunDescriptor`
- `PromptAccepted` / `RootExecutionAccepted` / runtime duplicate receipt
- protocol `status: String`
- notification / DTO 外层重复 `open_session_id`
- duplicated `openable`
- `legacyDurable` 与 descriptorless downgrade source
- 没有当前消费者的 operator / skeleton surface

## 4. Enforcement

- 任一保留 surface 都必须能指向当前真实消费者与 owner boundary。
- 任何新增 surface 若没有 owner 和消费者，不得以“以后也许会用”为理由进入主线。
