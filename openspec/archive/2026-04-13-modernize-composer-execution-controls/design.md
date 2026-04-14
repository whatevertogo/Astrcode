## Context

当前 composer 相关能力存在两类明显断层：

- 用户发起的执行控制还停留在零散逻辑里，例如 busy 状态下 `/compact` 直接拒绝
- `maxSteps`、`tokenBudget` 已经在协议和前端留下 TODO，但还没有变成稳定合同

如果继续在前端本地堆判断分支，会破坏 “Server is the truth” 原则；如果不把这些控制提升为正式输入，又会让执行语义长期依赖隐式默认值。

## Goals / Non-Goals

**Goals:**

- 建立稳定的 composer 执行控制模型，让前端通过 HTTP/SSE 合同表达控制，而不是本地猜测
- 让根代理执行、子代理执行和普通会话 turn 共享一致的控制字段语义
- 让手动 compact 在运行中状态下有正式处理路径，而不是纯前端硬拒绝
- 保持协议层为纯 DTO，不把执行策略塞进前端或 mapper

**Non-Goals:**

- 不实现附件上传、文件选择或富输入能力
- 不做大规模视觉重构
- 不引入通用命令脚本队列或任意任务编排器

## Decisions

### 决策 1：把执行控制建模为显式 DTO，而不是继续从文本中猜

新增统一的执行控制输入结构，用于承载：

- `tokenBudget`
- `maxSteps`
- 手动 compact 等控制命令

前端负责表达用户意图，`protocol` 负责 DTO 承载，`application` 负责校验和归类，`session-runtime` 负责 turn 内预算语义。这样能够避免把 `/compact`、预算参数和后续控制能力散落在不同入口里。

备选方案是继续通过 slash command 文本与 TODO 字段逐步补洞；该方案被放弃，因为它会使前端和后端长期共享隐式解析约定。

### 决策 2：运行中状态下的手动 compact 走“延迟执行控制”，不留在前端本地排队

当 session 正在执行时，手动 compact 不再仅由前端拒绝，而是作为一种受控的 session-level control request 进入服务端真相面，在当前 turn 结束后按规则执行。

这样做的原因是：

- 是否真正执行 compact 属于业务事实，应由 server 决定
- 控制请求需要与 session 生命周期一致，前端本地队列不可靠
- 后续若增加更多控制命令，可以沿用同一模型

备选方案是仅在前端维护一层本地命令排队；该方案被放弃，因为窗口刷新、会话切换或重连都会让本地排队语义变脆弱。

### 决策 3：`tokenBudget` 和 `maxSteps` 采用“可选覆盖默认值”的输入模式

执行控制字段均为可选：

- 未提供时，系统继续使用现有默认配置
- 提供时，由 `application` 做参数校验并显式传入业务边界
- `session-runtime` 对 `tokenBudget` 负责 turn 内自动续写决策
- 根代理执行/子代理执行入口对 `maxSteps` 负责执行上限语义

这样既能保持兼容现有调用方，也能避免控制参数漂浮在前端 TODO 中无人消费。

### 决策 4：同一控制语义通过不同入口共享一套校验规则

普通会话 prompt、根代理执行和子代理执行都可能使用预算/步数控制，因此需要共享统一规则：

- DTO 字段名一致
- 非法值在 `application` 层拒绝
- 未生效或被延迟执行时，错误/状态通过稳定业务结果返回

这比为每条入口各自增加一套参数解析更符合长期架构。

## Risks / Trade-offs

- [Risk] 手动 compact 延迟执行会让用户感知与当前立即失败不同 → Mitigation：通过前端状态提示明确“已登记，当前 turn 结束后执行”
- [Risk] 在多个入口复用控制字段时容易出现语义不一致 → Mitigation：把字段规范下沉到 shared DTO 与 spec，而不是复制注释
- [Risk] `maxSteps` 的执行上限若只在部分入口生效，会制造混淆 → Mitigation：在 spec 中明确每个入口的生效范围和失败路径
- [Risk] 过早引入过多控制字段会污染协议 → Mitigation：本次只收口已明确存在的 `tokenBudget`、`maxSteps` 和手动 compact

## Migration Plan

1. 在 protocol 中引入统一执行控制 DTO，并把现有 TODO 字段替换为正式合同
2. 在 `application` 中增加共享校验与路由逻辑
3. 在 `session-runtime` / agent 执行入口中接入控制参数
4. 将前端 composer 从本地硬拒绝迁移为提交显式控制请求
5. 更新错误处理和状态提示，确保用户能理解延迟执行或参数拒绝原因

回滚策略：

- 若延迟执行控制路径不稳定，可先保留 DTO 与参数校验，只暂时关闭“运行中 compact 延迟执行”，退回显式拒绝
- 若新控制字段导致兼容问题，可在前端隐藏入口，同时保留服务端默认行为

## Open Questions

- 手动 compact 的延迟执行是否需要写入事件日志，还是由 session 内部控制状态即可表达
- `maxSteps` 对根执行和子执行的默认值是否完全一致，还是允许不同入口有不同默认上限
- 前端是否需要为“待执行的控制请求”显示专门 UI，还是先用现有本地提示承载
