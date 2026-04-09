# Design: Runtime Boundaries

## Goals

- 为独立子会话、resume、父唤醒和 lineage 恢复失败指定单一 owner。
- 明确 durable 真相、运行时桥接和 `/history`、`/events` 投影之间的边界。
- 在不引入新服务或新 durable 存储的前提下完成切换。
- 删除旧共享写入模式的读取、回放与恢复路径。

## Non-Goals

- 不新增跨进程 durable 消息队列。
- 不在本 feature 内扩展新的用户级协作能力集合。
- 不保留旧共享写入模式的兼容读取或投影。

## Boundary Ownership

| Boundary | Owner | Responsibilities | Must Not Do |
|----------|-------|------------------|-------------|
| `storage` + `runtime-session` | 会话 durable truth | child session JSONL、会话加载、replay、legacy 共享历史显式拒绝 | 不负责父唤醒、prompt 裁剪或前端 UI 状态 |
| `runtime-execution` | 编排与边界事实 | spawn/resume 流程、lineage 关系构建、父侧边界事实写入、context snapshot 分离、wake turn 调度 | 不持有长期 durable 队列，不直接实现前端投影 |
| `runtime-agent-control` | 运行时控制与缓冲 | 父唤醒排队、一次性交付输入缓冲、幂等消费、活跃 agent registry | 不伪装成 durable 真相来源，不跨进程持久化交付 |
| `runtime-agent-loop` | 活跃 turn 消费 | 接收唤醒、消费一次性交付输入、把当前 turn 与缓冲桥接起来 | 不把运行时唤醒消息写回 durable transcript |
| `runtime-prompt` | prompt 继承与缓存 | inherited blocks、fingerprint、LayerCache、builder version 边界 | 不关心父子 durable lineage |
| `protocol` + `server` | 协议与投影 | `/history`、`/events`、status DTO、父摘要视图数据、legacy 显式错误投影 | 不携带 runtime 内部队列或 prompt builder 内部细节 |
| `frontend` | 读模型呈现 | 父摘要展示、子会话入口、错误显式展示 | 不从混合历史里重新猜 lineage 真相 |

## Implemented Cutover Notes

- 父唤醒用到的 `tokio::spawn` 句柄已经由 runtime service 统一收集，避免“发出去就丢”的 unmanaged spawn。
- 父交付队列仍然是进程内结构，但队列变更都限制在单个锁作用域内完成；运行时异步工作发生在锁外。
- `session_load_lock` 已切到 owned lock，避免唤醒链路在异步装载 child/parent session 时把非 `Send` 锁守卫带过 `.await`。

## Flow 1: 新子智能体创建

1. 父智能体触发 `spawnAgent` 或等价协作入口。
2. `runtime-execution` 创建新的 `ChildSessionNode` 和首个 `ChildExecutionInstance`。
3. `runtime-session` / `storage` 为新 `child_session_id` 创建独立 durable 历史。
4. `runtime-execution` 写入父侧 `ParentChildBoundaryFact(kind=started)`。
5. `runtime-prompt` 为 child 构建任务消息与 inherited prompt blocks。
6. `runtime-agent-loop` 启动 child 执行。

**Key invariants**

- 新子智能体必须拿到新的 `child_session_id`。
- 父历史只看到 started 边界事实和后续摘要，不混入 child transcript。

## Flow 2: Resume

1. 调用 resume 入口时，`runtime-execution` 先定位现有 `ChildSessionNode`。
2. `runtime-session` 基于 child session durable 历史做 replay 或 projector 恢复。
3. 若恢复得到完整 `ResumeVisibleState`，则创建新的 `ChildExecutionInstance(trigger_kind=resume)`。
4. `runtime-execution` 写入 `ParentChildBoundaryFact(kind=resumed)`。
5. 若 lineage 不一致或历史损坏，则写入父侧边界错误事实和系统诊断日志，resume 失败返回。

**Key invariants**

- resume 成功时 `child_session_id` 不变、`execution_id` 必变。
- 任何 unsafe restore 都必须失败，而不是降级成 spawn。

## Flow 3: 子交付与父唤醒

1. child 进入 `delivered`、`completed`、`failed`、`cancelled` 等终态或交付点。
2. `runtime-execution` 先写入父侧 `ParentChildBoundaryFact`，确保 durable 可追溯。
3. `runtime-agent-control` 创建对应 `PendingChildDelivery` 并排队。
4. 若父当前空闲，runtime service 调度 wake turn，并把 spawn handle 收进统一句柄注册表。
5. 父下一轮消费 `PendingChildDelivery`，构建一次性交付输入，消费后清除。

**Key invariants**

- 父唤醒依赖运行时信号，不依赖 durable `UserMessage`。
- 若父忙碌，多个交付必须独立缓冲、逐个消费。
- 若进程在消费前重启，缓冲可消失，但 `ParentChildBoundaryFact` 与 child session 入口必须保留。
- SSE / `/history` / `/events` 只能看到 durable boundary facts，不能看到 runtime wake turn 的内部桥接材料。

## Flow 4: Legacy Shared History Rejection

1. 调用方尝试读取、回放或恢复旧共享写入历史。
2. `runtime-session` / `server` 识别出不受支持的共享写入 durable 语义。
3. 系统返回稳定错误码 `unsupported_legacy_shared_history` 与 `upgrade_required` 或 `cleanup_required` 动作提示。

**Key invariants**

- 不再为旧共享写入历史构造新的 lineage、projection 或 resume 语义。
- 显式拒绝优先于“尽力猜测”或保留双轨路径。

## Flow 5: Lineage Query 与 Projection

1. lineage 查询基于 `ChildSessionNode`、`ChildExecutionInstance` 和 `ParentChildBoundaryFact` 组合重建。
2. `server` 把它们投影成 `/history`、`/events` 和 status DTO。
3. `frontend` 直接使用投影，不重新从父消息体猜 child 内部历史。

**Key invariants**

- lineage 是查询模型，不引入新的全局双写真相表。
- `/history` 与 `/events` 必须对同一边界事实语义一致。

## Error And Observability Design

- child create、resume、cache hit/miss、delivery queued/consumed、lineage mismatch、legacy rejection 都必须打结构化日志。
- `lineage_mismatch` 必须同时体现在父侧边界事实和系统诊断日志中。
- `unsupported_legacy_shared_history` 必须拥有稳定错误码和清晰推荐动作。
- SSE lagged replay 恢复失败必须发结构化 error event，而不是只在服务端日志里静默结束。
- 不允许用“过滤掉 history 显示”来替代 durable 层问题修复。

## Cutover Rule

- 新写入路径不再允许回落到共享写入模式。
- 旧共享写入历史不再可读、可回放、可恢复。
- 若仓库或用户环境仍存在 legacy 数据，必须通过外部升级或清理流程处理，而不是在 runtime 中保留兼容逻辑。
