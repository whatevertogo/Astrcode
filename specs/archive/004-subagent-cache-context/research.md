# Research: 子智能体会话与缓存边界优化

## Decision 1: 新子智能体默认使用独立子会话，并显式拒绝旧共享写入历史

**Decision**  
所有新创建的子智能体默认进入独立 child session durable 模型；旧共享写入历史不再属于支持范围，系统遇到此类数据时必须明确返回 `unsupported` 或 `upgrade required`，而不是继续提供读取、回放或恢复兼容路径。

**Rationale**  
这是和“干净架构优先、不做向后兼容”直接对齐的 cutover。只要还保留旧共享写入历史的读取/回放/恢复逻辑，`runtime-session`、`server`、投影和测试就必须长期维护双轨 durable 语义，边界永远收不干净。

**Alternatives considered**

- 继续保留 legacy 只读兼容：会让同一仓库长期背两套 durable 语义。
- 在本 feature 内做一次性历史迁移：风险高、范围大，而且规格已经允许直接拒绝 legacy 数据。

## Decision 2: Resume 必须基于 child session durable replay 恢复可见状态

**Decision**  
resume 的成功语义是“沿用原 child session identity，并基于 child session durable 历史 replay 或等价 projector 恢复下一轮执行所需的完整可见状态”。规格只约束恢复语义，不强绑某个内部状态结构。

**Rationale**  
当前最危险的偏差不是“session id 没沿用”，而是“session id 看起来沿用了，但实际执行从空状态重开”。只有把 replay/projector 恢复设为强语义，resume 才能真正与原子会话连续。

**Alternatives considered**

- 只要求沿用 `session_id`，允许内部从空状态重建：会伪装成恢复，实际仍是重开。
- 强制把 compaction/recovery 材料都塞进单一 `AgentState`：会把现有分层恢复设计硬绑到单个结构上，和仓库边界不一致。

## Decision 3: 父唤醒通过运行时信号完成，durable log 只记录边界事实

**Decision**  
父唤醒不再通过向 durable 历史写入 `ReactivationPrompt` 或等价机制性 `UserMessage` 触发，而是通过运行时信号与一次性交付输入完成；durable 只保留 `delivered`、`completed`、`failed`、`cancelled` 等边界事实。

**Rationale**  
durable JSONL 应记录事实，而不是运行时机制。把机制消息写入 durable 既污染父历史，也会破坏 prompt cache 连续性，还会让 replay 结果和真实业务语义混在一起。

**Alternatives considered**

- 继续写 durable `ReactivationPrompt`，只是在 history 投影里过滤：问题被隐藏了，但 durable 真相仍然不干净。
- 引入新的 durable “wake requested” 机制事件：仍然是在 durable 层记录运行时桥接，而不是业务事实。

## Decision 4: 父背景通过 `PromptDeclaration -> system blocks` 传给子智能体

**Decision**  
`resolve_context_snapshot()` 只产出任务主体；父的 compact summary 与 recent tail 通过 `PromptDeclaration` 注入独立 system blocks，不进入子消息流。

**Rationale**  
消息流应该表达任务目标，prompt 层应该表达继承背景。只有这样才能同时做到任务消息清晰、背景分层更新、低频稳定层可缓存、高频 recent tail 可单独变化。

**Alternatives considered**

- 继续把 summary/tail 拼到首条 `UserMessage`：实现简单，但会继续污染消息语义与缓存边界。
- 把继承背景写成新的 durable 事件，再在 replay 时还原：会把 prompt 结构问题转嫁成 durable 历史问题。

## Decision 5: 缓存复用行为委托给 `runtime-prompt` fingerprint 与共享 LayerCache

**Decision**  
规格只要求“相同可观察输入必须命中、相关输入变化必须失效”，具体强指纹计算委托给 `runtime-prompt` 现有 fingerprint 体系；实现通过共享 LayerCache 或等价机制复用稳定上下文准备结果。

**Rationale**  
仓库里已经有覆盖 working dir、profile、工具、rules、prompt declarations、skills、builder version 等维度的 prompt fingerprint 机制。规格层再定义一套 key 规则只会制造双重真相。

**Alternatives considered**

- 在 spec 里手写固定字段哈希规则：容易过时，也无法完整覆盖 prompt builder 的真实输入。
- 只按长度或条目数量复用：属于弱特征，会造成难排查的误命中。

## Decision 6: Recent Tail 默认采用确定性筛选，不新增额外推理回合

**Decision**  
recent tail 的整理默认采用角色优先级、token budget 裁剪、工具输出摘要和长文本截断等确定性规则；除非后续规格另行声明，不为了构建 recent tail 再发起额外 LLM 推理。

**Rationale**  
recent tail 属于高频变化层，如果再引入额外推理，不但会扩大延迟和成本，还会把本该简单可预测的 prompt 继承变成新的不稳定依赖。

**Alternatives considered**

- 每次都调用模型先“理解 recent tail 再传给 child”：信息质量可能更高，但成本、延迟和可预测性都明显更差。
- 完全不整理 recent tail，原文硬塞：会直接拖垮 token 预算和缓存稳定性。

## Decision 7: 多子交付采用“进程内可靠送达 + durable 可追溯”的双层语义

**Decision**  
在同一进程存活期间，多个子交付必须独立缓冲、逐个送达、幂等消费；若进程在消费前重启，运行时缓冲允许丢失，但 durable 边界事实和子会话入口必须保留，以便后续追溯和人工/自动补偿。

**Rationale**  
当前运行时缓冲本质上是内存态，不适合伪装成“跨重启可靠队列”。与其承诺过强语义，不如明确区分“运行时可靠送达”和“重启后 durable 可追溯”两层保证。

**Alternatives considered**

- 把所有待消费交付都做成新的 durable 队列表：复杂度高，会把本次 feature 扩成新的消息系统。
- 完全依赖内存缓冲且不写 durable 边界事实：一旦重启就失去交付可追溯性。

## Decision 8: SC-003 以 provider 的缓存指标能力为验收前提

**Decision**  
`SC-003` 只在支持缓存指标的 provider 上使用 `cache_creation_input_tokens` 验收；不支持该字段的 provider 必须在 feature 交付前补齐等价可观察指标，否则暂不纳入本条验收范围。

**Rationale**  
仓库当前不同 provider 的 telemetry 能力不一致。如果不显式限定验收环境，这条成功标准会因为“字段恒为 0”而失去可验证性。

**Alternatives considered**

- 对所有 provider 强行使用同一指标：会让不支持缓存 telemetry 的 provider 得到假阴性或假阳性结果。
- 直接退化为耗时指标：更容易受机器、网络和 provider 波动影响，不适合作为主成功标准。
