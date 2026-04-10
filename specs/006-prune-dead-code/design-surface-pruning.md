# Design: 支持面清点与 Surface Pruning

## 1. 目标

本设计回答三个问题：

1. 哪些 surface 仍属于当前正式支持面？
2. 哪些 surface 可以立即删除？
3. 哪些 surface 必须先迁移当前调用方，再删除？

这里的 surface 包括：

- 前端导出 API / projection / 类型
- server public HTTP route
- runtime / protocol 公开语义
- 当前 live 文档与测试对外宣称的能力

## 2. 分类规则

### 2.1 明确保留

满足以下全部条件：

- 当前产品流程真的会走到
- 能明确说出 owner boundary
- 删掉会破坏主线能力
- 没有更窄、更唯一的替代入口已经存在

### 2.2 立即删除

满足以下任一条件即可：

- 没有当前消费者
- 只剩测试、夹具或文档引用
- 明确是“未来入口”“预实现”“骨架接口”
- 与当前主线能力重复表达同一语义

### 2.3 迁移后删除

满足以下条件：

- 设计上已过时
- 但当前主线流程仍在使用
- 已存在可切换的唯一替代路径

## 3. Surface Inventory

### 3.1 明确保留

| Surface | Why kept | Owner |
|--------|----------|-------|
| 会话历史 / SSE 主线 | 当前会话浏览、消息提交、增量事件消费依赖它们 | `server` + `frontend` |
| `activeProjectId` / `activeSessionId` / `activeSubRunPath` 导航状态 | 当前前端视图切换主线 | `frontend/store` |
| `buildSubRunThreadTree` 与 focused subrun 过滤 | 当前子执行聚焦浏览仍在使用 | `frontend/lib` |
| `SubRunHandoff.summary` | 当前子 Agent 终态交接摘要 | `core/runtime/frontend` |
| `ChildSessionNotification.summary` | 当前父侧通知摘要与 child session 入口 | `core/server/frontend` |
| 当前配置读写与模型选择相关 surface | Settings 仍在使用 | `server` + `frontend` |
| 当前子会话直开能力 | 当前 UI 能从通知/消息中直接打开 child session | `frontend` + `server/history` |

### 3.2 立即删除

| Surface | Why remove | Notes |
|--------|------------|-------|
| `loadParentChildSummaryList` | 无当前 UI 调用 | 同步删 server route |
| `loadChildSessionView` | 无当前 UI 调用 | 同步删 server route |
| `buildParentSummaryProjection` / `ParentSummaryProjection` / `ChildSummaryCard` | 只剩测试与文档自证 | 不影响当前 child session 直开 |
| `/api/sessions/{id}/children/summary` | 无当前消费者 | 与现有消息/通知重复 |
| `/api/sessions/{id}/children/{child_session_id}/view` | 无当前消费者 | 与直接打开子会话重复 |
| `/api/v1/agents` + `/api/v1/agents/{id}/execute` | 无当前产品入口 | 不再保留“外部 agent API”错觉 |
| `/api/v1/tools` + `/api/v1/tools/{id}/execute` | 无当前产品入口；execute 还是骨架 | 直接删除 |
| `/api/runtime/plugins` + `/api/runtime/plugins/reload` | 无当前产品入口 | 不保留 operator 幻觉 |
| `/api/config/reload` | 无当前产品入口 | 当前设置面只需读/写配置 |

### 3.3 迁移后删除

| Surface | Current caller | Replacement |
|--------|-----------------|-------------|
| `cancelSubRun()` 前端包装 | `SubRunBlock -> Chat -> App -> useAgent` | `closeAgent` 协作能力 |
| `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel` | 当前 UI 的取消按钮 | `closeAgent` 执行路径 |
| 与 cancel route 绑定的 server/protocol tests | 证明 legacy cancel 入口存在 | 新主线 cancel 行为测试 |

## 4. Summary 收口原则

### 4.1 保留

- 子 Agent 的 terminal handoff summary
- 父侧 child notification summary

### 4.2 删除

- 没有消费者的 parent summary projection
- 仅靠额外 endpoint 才能获取、但当前 UI 已不需要的 summary 视图

### 4.3 不允许的做法

- 因为某个 summary projection 要删，就把所有 `summary` 字段一起删掉
- 因为某个 summary 字段有用，就把所有同名 projection 一起保活

## 5. 测试与文档约束

- 每删一个 surface，都必须删掉“只为证明它存在”的测试。
- live 文档中不再出现已删除 surface 的“stable / experimental / future”描述。
- archive 文档可以保留，但不得继续从 live spec 链接它们来描述当前事实。

## 6. Guardrails

- 不能删掉当前 focused subrun 浏览辅助树，只能删掉其中的 orphan / legacy 分支。
- 不能在迁移 `cancelSubRun` 时留下第二条长期主线入口。
- 不能把“当前没前端按钮”自动等于“可删”；如果是明确 operator 契约，也需要 owner 和用途说明，否则仍按死代码处理。
