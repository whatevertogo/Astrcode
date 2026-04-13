## Context

当前系统在 agent 执行入口上存在两套彼此漂移的事实源：

- server 根执行路由直接 `create_session + submit_prompt`，没有走 `application::execution::execute_root_agent`
- `application::agent::launch` 为 subagent 现场拼装占位 `AgentProfile`，没有消费 working-dir 解析出的真实 profile

这使得既有 specs 中承诺的 profile 存在性校验、mode 校验、工具约束与模型偏好，无法稳定作用到真实 root/subagent 执行。与此同时，`ProfileResolutionService` 已经存在，但没有成为执行主链的正式依赖，导致 watch、delivery、observability 等后续 change 都失去可靠前提。

## Goals / Non-Goals

**Goals:**

- 让 root execution 与 subagent execution 共享统一的 profile 解析事实源
- 收口 server 路由，使执行入口只能通过 `application` 正式用例进入
- 让 `ProfileResolutionService` 成为执行主链依赖，而不是孤立缓存服务
- 恢复 profile 语义对执行的正式影响，包括存在性、mode、工具限制与模型偏好

**Non-Goals:**

- 不处理 child delivery 到 parent wake 的接线
- 不引入新的 profile 文件格式或新的 adapter-agents 搜索规则
- 不在本 change 中实现文件监听与自动失效

## Decisions

### 决策 1：root 执行只允许通过 `application::execution::execute_root_agent`

根执行路由不再自行创建 session 并提交 prompt，而是只负责 DTO 转换与错误映射，统一委托 `application` 正式入口。

原因：

- 这符合 `PROJECT_ARCHITECTURE.md` 中“application 是唯一用例入口”的长期约束
- 只有这样才能让 root 执行与 spec 对齐，并确保 root agent 注册、profile 校验与执行控制消费出现在同一业务边界

备选方案：

- 保留路由层直提 prompt，仅在路由中补 profile 校验  
  不采用，因为这会继续让 server 持有业务语义，破坏分层

### 决策 2：subagent 启动必须先解析真实 profile，再进入 child session 创建

`spawn` 的业务顺序调整为：

1. 用 working-dir + profile id 解析真实 profile
2. 校验 profile 是否允许作为 subagent
3. 基于该 profile 构建执行请求
4. 再创建 child session 并提交 prompt

原因：

- profile 必须是执行输入，而不是仅用于 prompt facts 或 UI 展示
- 先解析 profile 可以避免“无效 agent 仍然创建 child session”的脏状态

备选方案：

- 保留现有最小 profile 占位结构，只在后续 turn 中补约束  
  不采用，因为这样工具约束、模型偏好与 system prompt 仍无法稳定生效

### 决策 3：`ProfileResolutionService` 负责 `application` 侧事实收口，adapter-agents 只做加载

`ProfileResolutionService` 将成为 root/subagent 执行入口的正式依赖；adapter-agents 继续只提供磁盘加载实现，不直接参与业务流程编排。

原因：

- 这保持了 `application -> core + kernel + session-runtime` 的依赖方向
- 也为后续 watch invalidation change 留出清晰插槽

备选方案：

- 让 `server/bootstrap/prompt_facts` 或 `adapter-agents` 自行承担缓存  
  不采用，因为会再次制造平行事实源

### 决策 4：执行入口对 profile 语义恢复为显式业务错误

执行入口必须对以下场景直接失败：

- profile 不存在
- profile mode 不允许对应执行类型
- profile 解析失败

原因：

- 这些都属于业务输入无效，不应继续下推到 `session-runtime`
- 显式失败也有利于 route、前端与 observability 对齐

## Risks / Trade-offs

- [Risk] 现有根执行接口可能依赖“任意 agent_id 都能启动”这一宽松行为  
  → Mitigation：在 proposal/spec 中将其标记为收紧后的正式合同，并让路由返回明确业务错误

- [Risk] root/subagent 都改为使用 resolver 后，隐藏的 profile 解析问题会集中暴露  
  → Mitigation：在设计中明确错误分类，并在实现时补充失败路径测试

- [Risk] 若 `ProfileResolutionService` 接线方式不当，可能与后续 watch invalidation change 冲突  
  → Mitigation：当前只定义 resolver 作为正式事实源，不在本 change 中耦合监听机制

## Migration Plan

1. 先让 server 根执行路由改为委托 `application` 正式入口
2. 在 `application` 中为 root/subagent 引入统一的 profile 解析路径
3. 删除 subagent 启动中的占位 profile 构造逻辑
4. 补齐 route、application、execution、profile resolution 相关测试

回滚策略：

- 若统一入口引入阻断性问题，可临时恢复旧路由转发，但必须同时撤回相关 spec 变更

## Open Questions

- root 执行是否需要在未来支持“复用已有 session”而非总是创建新 session；本 change 先保持现有“新建 session”语义不变
