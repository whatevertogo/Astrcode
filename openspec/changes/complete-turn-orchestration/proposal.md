## Why

当前 `session-runtime` 已经具备多 step 的 turn 循环和上下文压缩管线，但 token budget 驱动的 auto-continue、turn 级 observability 汇总和 compaction tail 的最终形态还没有闭环。现在补齐这一层，是为了把旧 runtime 中最核心的单 session 执行体验完整迁入，同时不破坏现有的 `session-runtime -> kernel -> application` 边界。

## What Changes

- 在 `session-runtime` 内补齐 token budget 到 auto-continue 的完整闭环，包括预算解析、继续/停止决策、continue nudge 注入和续写上限控制
- 将 turn 级 observability 从“零散事件”提升为稳定汇总结果，覆盖 prompt cache reuse、turn 耗时、continuation 次数和 compaction 命中
- 明确 compaction tail 的目标形态：如果现有 `recent_turn_event_tail` 已足够，则固化为正式契约；如果不足，再补显式快照结构
- 保持 `application` 只负责发起一次 turn，不下沉 turn 内的循环、裁剪、budget 或 observability 细节
- **BREAKING**：`submit_prompt` 触发的单次 turn 可能在预算允许时自动产生多轮 LLM 续写，相关事件序列和运行时指标会更丰富

## Capabilities

### New Capabilities
- `turn-budget-governance`: 定义 token budget、auto-continue 和续写停止条件的行为契约
- `turn-observability`: 定义 turn 级执行指标、cache reuse 和 compaction 命中的稳定汇总契约

### Modified Capabilities
- `session-runtime`: 扩展单 session turn 执行要求，使其包含 budget 决策、observability 汇总和 compaction tail 语义
- `application-use-cases`: 调整 `App::submit_prompt` 的要求，明确它只触发 turn，不拥有 turn 内循环策略

## Impact

- 受影响 crate：`crates/session-runtime`、`crates/application`
- 受影响模块：`turn/runner.rs`、`turn/token_budget.rs`、`context_window/request_assembler.rs`、`application/src/lib.rs`
- 用户可见影响：单次 prompt 的事件流可能包含自动续写产生的更多 assistant/tool 事件
- 开发者可见影响：turn 预算和执行指标将有更稳定的调试与治理出口
- 迁移与回滚：迁移按“先补行为，再替换旧逻辑”进行；如需回滚，可先停用 auto-continue 判定，保留当前多 step 基础循环
