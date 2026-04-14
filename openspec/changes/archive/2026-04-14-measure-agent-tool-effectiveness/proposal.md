## Why

当前 Astrcode 对 agent-tool 的优化主要依赖体感和个案观察：我们知道某些 prompt 会导致过度 `spawn` 或无效 `observe`，但没有一套正式的评估面来回答“这次调整到底有没有减少浪费、提升交付价值”。Claude Code 的源码显示，它会持续记录 task progress、tool summary、permission/tool 决策和 feature-gated 实验；Astrcode 也需要一套与自身事件日志和 server truth 相容的评估系统，而不是继续靠直觉调 prompt。

## What Changes

- 新增 `agent-tool-evaluation` capability，定义 agent-tool 的原始协作事实、派生效果指标和可供调优的稳定读模型。
- 修改 `runtime-observability-pipeline` capability，将 agent collaboration 诊断纳入正式 observability 管线，而不是只留在零散日志中。
- 修改 `turn-observability` capability，使每个 turn 都能产出稳定的 collaboration summary，用于回答“这一轮到底有没有通过 child reuse、delivery 或 close 获得收益”。
- 为协作事实补充策略上下文，如 prompt/policy revision、深度限制、fan-out 限制等，使后续实验比较具备可解释性。
- 定义最小有效的效果指标集合，例如 child reuse ratio、observe-to-action ratio、spawn-to-delivery ratio、orphan child ratio、delivery latency 与 spawn rejection diagnostics。

## Capabilities

### New Capabilities

- `agent-tool-evaluation`: 定义 agent collaboration 原始事实、派生效果指标、策略上下文标记与稳定评估读模型。

### Modified Capabilities

- `runtime-observability-pipeline`: observability 管线必须正式采集并暴露 agent collaboration 诊断，而不是只覆盖一般 turn/subrun 执行。
- `turn-observability`: turn 汇总必须纳入 collaboration summary，帮助开发者判断该轮 agent-tool 是否创造了真实价值。

## Impact

- 影响代码：
  - `crates/application/src/observability/*`
  - `crates/application/src/agent/*`
  - `crates/session-runtime/src/turn/*`
  - `crates/server/src/bootstrap/*`
  - `crates/server/src/http/*` 或治理/调试读取面
  - 相关 adapter-prompt / adapter-tools（用于标记 policy revision 或协作上下文）
- 用户可见影响：
  - 后续可以在治理或调试视图中看到更靠谱的 agent-tool 效果诊断
  - prompt / runtime 调优不再完全依赖主观体感
- 开发者可见影响：
  - agent-tool 调整将拥有统一的事实源和派生指标
  - 后续做 prompt A/B、fan-out 调整或 observe 语义调优时，有正式读模型可验证

## Non-Goals

- 不引入第三方 analytics 平台作为本次设计前提。
- 不在本 change 中做 LLM 评审式的主观质量打分或奖励模型。
- 不把评估系统做成完整产品 BI 平台；本次只建立工程可用的事实与读模型闭环。
