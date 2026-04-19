## Why

当前 `session-runtime` 对工具反馈的主要优化手段是 `micro_compact` 和 `prune_pass`：旧结果被清掉、长结果被截断、必要时再靠 file recovery 补回上下文。这对于控 token 是有效的，但对 agent loop 的“下一轮能否读到高质量反馈”帮助仍然有限，模型经常只能看到“旧结果被清了”的占位文本，而拿不到一段可直接推理的工具结论。

博客里另一条值得落地的思想是：工具反馈不是原样堆回上下文，而是先加工成更适合下一轮决策的反馈包。这里也要注意别误读 Claude Code：它的 `toolUseSummaryGenerator` 主要是面向 UI/SDK 的短摘要标签，不是下一轮 prompt 的 durable 真相。Astrcode 更适合做的是 prompt-local 的 `ToolFeedbackPackage`，把“供模型继续推理的高密度反馈”与“原始事件真相 / UI 摘要”明确分层。

## What Changes

- 为 `session-runtime` 增加 step-local 的工具反馈摘要/打包能力，把一批 `raw_results` 转成更适合下一轮 prompt 使用的反馈包
- 让 request assembly 优先消费工具反馈包，而不是被迫在“原始大结果”和“占位文本”之间二选一
- 保留原始 tool result 作为 durable 事实源与 replay 真相，不让摘要替代事件日志
- 明确区分 prompt-local 反馈包与未来可能存在的异步 UI/诊断摘要，避免两种语义混用
- 让工具反馈打包遵守 prompt budget、clearable tool 语义和现有 compaction/recovery 边界
- 为反馈打包增加稳定的 observability 与测试覆盖，便于判断它是否真的减少了无意义上下文噪音

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `turn-orchestration`: tool cycle 之后、下一轮 request 之前需要新增 step-local 工具反馈打包阶段
- `turn-budget-governance`: 工具反馈打包需要纳入 prompt budget 与 clearable tool 语义约束
- `turn-observability`: turn 汇总与诊断需要反映工具反馈打包命中情况

## Impact

- 受影响 crate：`crates/session-runtime`、`crates/core`
- 重点模块：`crates/session-runtime/src/turn/request.rs`、`crates/session-runtime/src/turn/tool_cycle.rs`、`crates/session-runtime/src/context_window/micro_compact.rs`、新增工具反馈打包模块
- 用户可见影响：多工具回合后，模型更容易拿到“可继续推理”的反馈，而不是只看到大量原始输出或清理占位文本
- 开发者可见影响：工具结果的“事实保存”和“提示词消费”将被正式拆成两层
- 迁移与回滚：先做 step-local 打包并保持原始结果完整；如果反馈包效果不好，可临时关闭 request 侧消费，退回到现有原始结果 + prune 策略
