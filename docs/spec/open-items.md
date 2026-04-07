# 开放项与待办

本文档收口原 `plan/` 与 `TODO/` 中仍然有效的未完成事项。

---

## 1. P0：协议与控制面收口

### 1.1 subrun 与 tool call 的稳定关联

- [ ] 为 `SubRunStarted / SubRunFinished` 增加 `tool_call_id`
- [ ] 如果短期不加字段，明确并实现“同一 `turn_id` 内按发出顺序 1:1 配对”

**影响面**：Agent as Tool、前端 subrun 卡片、历史回放

### 1.2 root-owned task control

- [ ] 设计并落地 root-owned task registry / task owner resolver
- [ ] 统一 `shared_session` 与 `independent_session` 的 kill / cleanup / timeout 通道
- [ ] 把任务 ownership 从 session mode 中剥离

**影响面**：shell、MCP、长任务、取消传播、回收责任

### 1.3 `/api/v1/tools/{id}/execute` 的去留

- [ ] 明确它是否继续成为正式执行入口
- [ ] 若保留，补齐边界与职责
- [ ] 若废弃，清理 501 骨架与相关文档

---

## 2. P1：读模型与前端导航增强

### 2.1 durable `list_subruns(session_id)`

- [ ] 评估是否需要 durable subrun read model
- [ ] 如需要，新增 `GET /api/v1/sessions/{id}/subruns`
- [ ] 该接口必须基于 durable 事件重建，再按需补 running 状态

### 2.2 server-side filter

- [ ] 评估 `subRunId + scope(self|subtree|directChildren)` 服务端过滤能力
- [ ] 只有在 history / events 载荷已成为瓶颈时再推进

### 2.3 `IndependentSession` 转正条件

- [ ] 明确从 experimental 升级为正式路径的准入条件
- [ ] 明确转正前必须具备的导航、控制面与可观测性能力

---

## 3. P1：Agent Profile 与子 Agent 能力补强

### 3.1 `model_preference`

- [ ] 决定 `AgentProfile.model_preference` 是正式实现还是删除
- [ ] 若保留，补齐 profile 级模型选择链路

### 3.2 `inherit_rules`

- [ ] 评估是否增加 profile 级 `inherit_rules`
- [ ] 若引入，明确它属于 context inheritance，而不是新的模糊 override

### 3.3 `isolation`

- [ ] 评估是否需要 `None / Worktree` 级别的隔离模式
- [ ] 若引入，必须说明生命周期、清理责任和与 git worktree 的关系

---

## 4. P2：Agent 协作能力

### 4.1 agent 间消息传递

- [ ] 评估 `SendMessage` / mailbox 能力
- [ ] 若引入，必须定义消息路由、顺序与失败语义

### 4.2 审批回根会话

- [ ] 评估需要审批的调用是否回路由到根会话
- [ ] 若支持，必须通过统一事件协议和控制面实现

### 4.3 shared observability aggregation

- [ ] 增强父对子执行的 step / token / outcome / findings / artifacts 聚合
- [ ] 明确 observability 与 shared mutable state 的边界

---

## 5. P1：Runtime surface 迁移收尾

- [ ] 迁移 `assemble_runtime_surface`
- [ ] 迁移 `prepare_scoped_execution`
- [ ] 迁移其他使用方
- [ ] 将 `RuntimeSurfaceContribution` 标记为 `#[deprecated]`
- [ ] 后续版本移除旧结构

---

## 6. P1：Compact 系统增强

### 6.1 Compact Hook

- [ ] 为 compact 增加 pre / post hook
- [ ] 保持 hook 语义收敛：block、prompt 增补、保留范围调整、自定义摘要、恢复操作

### 6.2 Prompt 工程升级

- [ ] 改进 compact prompt 的结构化程度
- [ ] 明确保留 goal / constraints / decisions / next steps
- [ ] 支持更稳定的增量重压缩 prompt

### 6.3 可审计 prune

- [ ] 用 prune 标记替代隐式删除旧工具结果
- [ ] 评估与 storage / replay / UI 的兼容方式

### 6.4 精确 token 计量

- [ ] 中期补更精确的 token 计量
- [ ] 评估多层预算保护与时间触发微压缩

---

## 7. P2：生态与扩展

- [ ] ACP 协议支持
- [ ] 动态工具注册与热重载
- [ ] 控制平面指标与失败分类

这些事项都不应先于主协议真相与控制面收口。

---

## 8. 实施边界

处理上述事项时必须保持：

- `spawnAgent` 公开 schema 继续极简
- `SharedSession` 仍是正式主线
- `SubRunFinished.result` 仍是父流程与 UI 的结果中心
- session tree 仍是 read model
- 优先增强 shared observability，不引入 shared mutable state
