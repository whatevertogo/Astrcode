## Context

旧项目里已经存在一套真实的 `RuntimeObservability` 采集器，但当前仓库只迁出了快照类型，尚未把采集器和治理接线恢复回来。结果是：

- `application` 能传递 `RuntimeObservabilitySnapshot`，但没有真实数据来源
- server 组合根返回默认零值快照
- turn / subrun / replay 等路径虽然已有部分聚合事实，但没有稳定汇入统一 observability 管线

这种状态会让状态接口看起来“类型正确”，实际却不能用于排障、回归和治理判断。

## Goals / Non-Goals

**Goals:**

- 恢复真实的 runtime observability 采集器，并接入当前 `application + session-runtime + server` 分层
- 让治理快照稳定暴露 session rehydrate、SSE catch-up、turn execution、subrun execution、delivery diagnostics 等指标
- 保持原始事件、turn 聚合结果和全局 observability 的职责分离
- 尽量复用现有快照 DTO，避免额外制造协议迁移成本

**Non-Goals:**

- 不为本次 change 设计新监控面板或外部导出系统
- 不把日志系统当作 observability 指标的替代品
- 不把所有 metrics 都提升为事件日志持久化事实

## Decisions

### 决策 1：观测采集器保留为独立组件，通过窄接口注入各层

具体实现采用“单一原子计数采集器 + 多个窄 recorder 接口”的形式：

- 采集器本体负责线程安全计数与快照生成
- `session-runtime`、`application`、server 恢复路径只依赖最小 recorder 接口
- 组合根负责把同一个采集器实例注入需要记录指标的协作者

这样可以避免 `session-runtime` 反向依赖 `application`，也能保持 observability 不是新的业务真相层。

备选方案是让每层各自记录再在治理时聚合；该方案被放弃，因为会增加重复计数和边界不清问题。

### 决策 2：turn 汇总继续作为中间聚合层，但治理快照读取统一 observability 采集器

`PromptMetrics`、`CompactApplied`、subrun 终态等原始事件仍然属于 session 事实；turn 汇总负责把单次 turn 的关键结果稳定投影出来；全局 observability 则负责跨 turn、跨 session 的累计计数。

这三层的关系是：

- 事件日志：真相
- turn 汇总：单轮稳定结果
- observability 快照：治理级累计视图

这样可以避免治理读取重复扫描整条事件流，也避免把治理计数直接写进事件日志。

### 决策 3：session rehydrate 与 SSE catch-up 指标在读路径上记录，不伪装成写侧事件

会话重水合和 SSE 回放本质上是读取行为，而不是业务事件，因此观测记录发生在对应查询/流恢复路径上，而不写入事件日志。

选择这种方式，是因为：

- 它们是运行时服务质量指标，不是业务事实
- 将其写入事件日志会污染 session 真相
- 采集器使用原子计数即可满足治理需求

### 决策 4：治理快照永远返回“当前已接线指标”，不能默认返回误导性零值

server 组合根不得再使用“默认零值观测器”作为常态实现。若某条指标链路尚未接线，治理快照必须通过显式字段策略表达“当前无该来源”，而不是伪装为真实零值。

在当前 change 中，优先策略是补齐已有指标链路，避免引入 tri-state 指标协议。

### 决策 5：delivery / subrun 诊断指标纳入统一快照，但不提升为独立治理子系统

子执行和 delivery buffer 的诊断计数已经与 `kernel` / `application` 协同紧密相关，继续放在同一个 observability 快照里即可，不额外创建新的治理组件。

备选方案是把 subrun diagnostics 拆成独立 snapshot provider；该方案被放弃，因为会让治理读取面再次分裂。

## Risks / Trade-offs

- [Risk] 采集点分布在多层，容易漏接一条路径 → Mitigation：为每类指标列出明确的 recorder 所属层，并补齐失败路径测试
- [Risk] 原子计数器无法表达复杂历史 → Mitigation：只用它承载治理累计值，细节分析仍依赖 turn 汇总和事件日志
- [Risk] 旧测试可能默认接受零值快照 → Mitigation：将测试从“类型存在”升级到“数值随行为变化”
- [Risk] 采集器接口过宽会污染边界 → Mitigation：只暴露最小记录方法，不把快照组装逻辑下沉到业务层

## Migration Plan

1. 定义并装配统一 observability 采集器及其窄接口
2. 把 session rehydrate / SSE catch-up / turn / subrun / delivery 现有记录点接回采集器
3. 让 `AppGovernance` 的 snapshot 从真实采集器读取，而不是默认零值
4. 调整 server mapper 与测试，验证治理快照反映真实指标
5. 清理旧占位实现与注释

回滚策略：

- 若某条新记录点导致不稳定，优先局部移除该记录点并保留采集器框架
- 若治理快照接线失败，应显式报错或禁用该指标来源，而不是回退到静默零值

## Open Questions

- 是否需要在本次 change 中同时恢复 plugin health probe 的观测计数，还是把它留给治理 reload change
- 是否要把 observability 采集器的 recorder trait 放在 `core`，还是放在更贴近运行时边界的 crate 中
- 某些长连接指标是否需要区分“本次请求失败”和“系统恢复成功但走慢路径”的不同严重度
