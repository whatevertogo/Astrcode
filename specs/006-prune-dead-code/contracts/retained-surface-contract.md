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

## 3. Runtime / Protocol

### 保留

- 当前会话与子会话历史主线
- 当前 handoff summary / child notification summary
- 当前 `closeAgent` 协作能力

### 不保留

- 只为 legacy downgrade 展示服务的公开状态语义
- 没有当前消费者的 operator / skeleton surface

## 4. Enforcement

- 任一保留 surface 都必须能指向当前真实消费者。
- 任何新增 surface 若没有 owner 和消费者，不得作为“以后也许会用”被引入。
