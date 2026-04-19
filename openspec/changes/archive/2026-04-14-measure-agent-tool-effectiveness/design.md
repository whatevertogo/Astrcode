## Context

Astrcode 已经开始对 agent-tool 做治理，例如：

- 限制子代理深度与每轮 fan-out
- 收紧 `spawn/send/observe/close` 的 prompt 契约
- 修正 child delivery 与 wake 的生命周期语义

但这些治理目前缺少一套正式的评估系统来回答几个关键问题：

- 限制 `spawn` 之后，是真的减少了浪费，还是同时压掉了本来有价值的并行？
- `observe` 的新结果形态，是否真的减少了重复轮询？
- child reuse 提升了没有？哪些 prompt policy 更容易产生 orphan child？

Claude Code 的源码表明，它把 agent/task 的 progress、summary、permission/tool decisions 和 feature-gated rollout 当作持续优化的一部分。Astrcode 不能直接照搬它的埋点系统，但可以借鉴它“先有事实，再做调优”的方法论，同时遵守本仓库“Server is the truth”和“事件日志优先”的约束。

## Goals / Non-Goals

**Goals:**

- 为 agent-tool 建立一套结构化、可重放、可聚合的原始事实层。
- 基于这些事实提供 turn 级与全局级的效果读模型。
- 让 prompt/runtime 策略调整可以与具体 revision 和配置上下文绑定，便于比较。
- 保持 DTO 纯数据结构，不把评估逻辑塞进协议层。

**Non-Goals:**

- 不把本次设计扩展成完整的商业 analytics 或可视化产品。
- 不引入依赖大模型再做一次“质量评估”的主观评分系统。
- 不让评估指标反向成为 runtime 的强制控制逻辑；本次只做诊断和决策支持。

## Decisions

### 决策 1：评估系统分成“原始协作事实”和“派生效果指标”两层

原始事实负责记录真正发生过什么，例如：

- `spawn` 尝试 / 成功 / 拒绝
- `send` 入队 / 被消费
- `observe` 调用与调用后的下一步动作
- `close` 级联结束
- child delivery 的到达、消费、重放与延迟

派生指标则回答“这些事实说明了什么”，例如：

- child reuse ratio
- observe-to-action ratio
- spawn-to-delivery ratio
- orphan child ratio
- delivery latency

替代方案是只记录最终聚合指标，但那样无法回放、也无法在规则变化后重新解释过去的行为。

### 决策 2：原始协作事实优先进入 server-side 持久化事实源，而不是只留在内存 collector

仓库明确要求“会话持久化优先基于事件日志，而不是隐式内存状态”。因此协作评估的原始事实应当成为 server-side 的正式记录，再由 observability collector 与 turn summary 读取和聚合。  
替代方案是像很多产品 analytics 一样只做内存计数或外发事件，但那会让本地调试、恢复和离线分析都缺少可重放事实源。

### 决策 3：策略上下文必须和协作事实一起被记录

单纯记录“发生了多少次 spawn”意义有限；要判断某次 prompt 或限制是否改善了系统，必须知道该行为发生在什么策略上下文中，例如：

- prompt / policy revision
- `max_subrun_depth`
- `max_spawn_per_turn`
- 当前工具集合或协作模式版本

替代方案是事后从代码版本或 git SHA 反推，但这对运行时读模型不透明，也不利于后续灰度实验。

### 决策 4：评估读模型分成 turn 级 summary 和全局 observability snapshot

两层读模型服务不同问题：

- turn 级 summary 用来回答“这一轮主代理到底如何使用了子代理”
- 全局 observability snapshot 用来回答“一段时间内这些策略是否在改善系统”

这与 Claude Code 里 task progress、summary 和 analytics 分层的思路一致，但 Astrcode 会把这两层都收口在 server truth 体系内。  
替代方案是只做全局计数，缺点是难以追到具体哪轮对话、哪类工具调用产生了问题。

### 决策 5：第一期只做最小诊断指标，不做主观质量评分

第一期指标只覆盖工程上可验证的行为：

- reuse / fan-out / orphan
- observe 后是否发生有效动作
- spawn 后是否产生有用 delivery
- delivery 的等待与处理延迟
- 配置/策略上下文与拒绝原因

替代方案是直接给 agent-tool 打一个“综合分”，甚至引入 LLM 复判结果质量；这样会快速失去可解释性，也不利于先把基础事实层做稳。

## Risks / Trade-offs

- [Risk] 事件量上升，给会话日志和聚合带来额外负担  
  Mitigation：只记录最小必要事实；派生指标在读侧聚合，不重复写大体积冗余数据。

- [Risk] 指标定义不当，导致团队把诊断指标误当成单一 KPI  
  Mitigation：在 spec 中明确这些是诊断读模型，不是质量总分，也不直接驱动 runtime 自动封禁。

- [Risk] 协作事实跨 `application`、`session-runtime`、`server` 三层，容易职责混乱  
  Mitigation：原始事实的记录点尽量靠近业务真相产生处，聚合逻辑统一进入 observability/summary 层，DTO 只负责承载纯数据。

- [Risk] policy revision 设计不清，导致实验结果不可比较  
  Mitigation：在实现前先定义稳定的 revision 来源和命名规则，至少保证同一轮评估内可比较。

## Migration Plan

1. 定义 `agent-tool-evaluation` spec，先收口原始事实与派生指标词汇表。
2. 在协作工具执行和 child delivery/wake 路径上追加原始事实记录点。
3. 扩展 turn summary 与 runtime observability snapshot，读取这些事实并生成稳定聚合结果。
4. 在治理或调试读取面暴露纯数据 DTO，供后续 UI 或脚本消费。
5. 回滚时可先停用新的读模型聚合；原始事实即使保留，也不会影响主业务执行。

## Open Questions

- policy revision 应该来自人工维护的语义版本，还是自动生成的 prompt/template 哈希？
- collaboration facts 最终应直接进入现有 session 事件流，还是使用专门的 sidecar/event category 更合适？
- 第一批读模型是先接治理快照，还是先做 debug-only 读取面更稳妥？
