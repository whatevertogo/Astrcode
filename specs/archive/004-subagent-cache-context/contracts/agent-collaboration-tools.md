# Contract: Agent Collaboration Tools

## 目的

收紧本 feature 影响到的子协作 surface，确保：

- 新 spawn 的 durable 语义稳定
- resume 不会伪装成重新 spawn
- 交付与父唤醒的运行时桥接不会泄漏成模型可见消息机制

本文件覆盖 tool surface 及其等价 runtime surface 的可观察行为，不约束内部 transport 细节。

## 1. `spawnAgent`

**Purpose**  
创建新的子智能体和新的独立 child session。

**Behavioral Guarantees**

- 成功时必须返回新的 `childSessionId` 或等价稳定子会话引用。
- 成功时必须返回新的 `executionId` 或等价执行实例引用。
- 父历史中只会新增 started 边界事实，不会混入 child 内部 transcript。

**Must Not**

- 不得复用已有 `childSessionId` 来伪装新 spawn。
- 不得通过向父 durable 历史写机制性消息来表示“child 已创建”。

## 2. `resumeAgent` 或等价恢复入口

**Purpose**  
恢复一个已经存在的 child session，使其继续沿用原会话身份工作。

**Behavioral Guarantees**

- 成功时必须沿用原 `childSessionId`。
- 成功时必须生成新的 `executionId`。
- 恢复所需上下文必须来自 child session durable replay 或等价 projector。

**Failure Contract**

- 若 lineage 不一致、历史损坏或恢复材料不足，必须明确失败返回。
- 失败时不得静默降级为新的 spawn。

## 3. 交付到父侧的运行时桥接

子交付结果进入父下一轮处理时，必须满足：

- 交付详情作为一次性结构化输入参与本轮处理
- 父 durable 历史只保留可追溯边界事实
- 父唤醒依赖运行时信号，而不是 durable `UserMessage`

## 4. 输出约束

所有协作 surface 的返回结果都必须是可消费语义，不得直接暴露：

- runtime 内部队列结构
- 原始 envelope
- prompt inheritance 内部 blocks
- durable 之外的临时实现细节

## 5. 身份与去重约束

- child targeting 必须依赖稳定 `childSessionId` / `childRef`，不能依赖 UI path。
- 同一交付或恢复请求必须具备幂等语义，避免重复消费。
- 父忙碌时收到多个交付，必须逐个缓冲、逐个送达，不得合并成单条模糊结果。
