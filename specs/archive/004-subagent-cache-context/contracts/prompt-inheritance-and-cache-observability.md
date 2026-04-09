# Contract: Prompt Inheritance And Cache Observability

## 目的

定义父传子背景、prompt 缓存边界和 telemetry 的可观察契约，确保实现后可以直接验证：

- 首条任务消息是否干净
- inherited context 是否走 prompt 层
- 缓存命中/失效是否遵循强指纹
- `SC-003` 是否可测

## 1. Context Split Contract

子智能体启动时，上下文必须拆成两部分：

1. `task payload`
2. `inherited context`

### `task payload`

- 进入消息流
- 只包含任务目标和必要直接上下文
- 不包含父 compact summary 或 recent tail 全文

### `inherited context`

- 通过 `PromptDeclaration` 注入 system blocks
- 至少包括 `compact_summary` 与 `recent_tail`
- 不得写成 durable `UserMessage`

## 2. Cache Boundary Contract

缓存复用必须遵循以下语义：

- 相同 working dir、profile、工具、规则、prompt declarations、skills、工具元数据和 builder version 等 prompt 输入时，可以复用稳定缓存
- 任一相关输入变化时，必须触发失效
- 不允许只凭长度、条目数量或其他弱特征做复用判断

具体 key 计算委托给 `runtime-prompt` fingerprint 体系，但行为必须满足上述约束。

## 3. Recent Tail Contract

- recent tail 必须经过确定性筛选和预算裁剪
- 大型工具输出必须压缩为摘要
- 不为了构建 recent tail 额外触发推理回合

## 4. Prompt Metrics Contract

### Supported Providers

- 必须产出 `PromptMetrics.cache_creation_input_tokens`
- 后续相似 child 启动的该指标相较首次下降至少 70%

### Unsupported Providers

- 必须补齐等价缓存可观测指标
- 在补齐前，不纳入 `SC-003` 验收范围

## 5. Must Not

- 不得把 inherited context 回写进 child durable transcript
- 不得把 cache hit/miss 判断建立在非强指纹上
- 不得用 history 过滤来掩盖 prompt 层边界污染
