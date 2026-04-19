## Context

当前 Astrcode 已经有两类与大工具输出相关的能力：

1. `adapter-tools` 内部的 per-tool 落盘：`readFile`、`grep`、`shell` 等工具在结果超过 `max_result_inline_size` 时会把内容写入 `~/.astrcode/projects/<project>/sessions/<session>/tool-results/`，并返回 `<persisted-output>` 文本引用。
2. `session-runtime` 内部的上下文清理：`micro_compact`、`prune_pass`、`auto_compact` 负责在 prompt 组装前清除旧工具结果或压缩上下文。

这意味着系统已经有“单个工具太大时外置”的基础设施，但缺少 Claude Code 真正关键的第二层：在 turn/request 阶段对“同一批 tool_result”做 aggregate budget，稳定决定哪些结果要外置引用、哪些必须保持原样，并将这个决策 durable 化，保证 resume / replay 后仍向模型展示完全一致的 replacement 文本。

本次设计不照搬 Claude Code 的 `query.ts` 代码形态，而是把其 `toolResultStorage + applyToolResultBudget` 思想翻译到 Astrcode 现有的 `turn/request`、`context_window`、`EventStore` 边界内。

## Goals / Non-Goals

**Goals:**

- 把 `<persisted-output>` 从工具局部实现提升为 turn/request 的正式 prompt contract
- 在 `assemble_prompt_request` 中引入 message-level aggregate tool-result budget
- 为 replacement decision 建立 durable truth，并支持 session 恢复后重建
- 复用现有 `readFile("tool-results/...")` 作为标准回读路径
- 为 replacement 命中、重放和节省量补充稳定 observability

**Non-Goals:**

- 不引入新的 UI/diagnostic summary 协议作为第一阶段主路径
- 不改变 `tool_result_persist` 的基础 `<persisted-output>` 文本契约
- 不新增新的外部 HTTP/SSE DTO
- 不把业务逻辑塞进 `CapabilityWireDescriptor` 或 protocol DTO
- 不要求所有工具都立刻支持新的 per-tool 自定义摘要器

## Decisions

### D1: aggregate tool-result budget 放在 `turn/request` 最前面，而不是工具执行侧

`adapter-tools` 继续只负责“单个工具自己的 inline limit”。  
真正的聚合预算属于 turn/request，因为它依赖：

- 当前 step 中已经进入消息序列的整批 `LlmMessage::Tool`
- 最终发给 LLM 的 API-level user message 分组语义
- prompt cache / replay 一致性要求

因此执行顺序固定为：

1. aggregate tool-result budget
2. `micro_compact`
3. `prune_pass`
4. prompt build
5. `auto_compact`

这样可以先把“应该外置引用的大结果”稳定降下来，再让后续 compaction 处理剩余上下文。

### D2: replacement state 以 `tool_call_id` 为主键，并作为 durable event 持久化

第一阶段不引入新的 sidecar 文件或隐式内存状态。  
replacement decision 必须进入现有事件真相链路，推荐新增：

- `StorageEventPayload::ToolResultReferenceApplied`

该事件至少包含：

- `tool_call_id`
- `replacement`：模型实际看到的完整 replacement 文本
- `persisted_relative_path`
- `original_bytes`

这样做的原因是：

- 恢复后可以 byte-identical 重放 replacement，避免 prompt cache 前缀漂移
- projection / replay 不需要额外读取临时 sidecar 决策文件
- durable truth 继续集中在 `EventStore`

### D3: aggregate budget 的选择策略固定为“fresh-only + largest-first”

在一个 API-level user tool-result 批次内，将候选分为三类：

- `must_reapply`：之前已经替换过，必须直接重放旧 replacement
- `frozen`：之前看过但没替换，后续不能再补替换
- `fresh`：当前第一次进入预算判断，可以新做替换决策

第一阶段选择策略固定为：

- 仅对 `fresh` 候选做新决策
- 从最大的 `fresh` 结果开始替换
- 直到该批次降到 budget 内，或 `fresh` 用尽

这是为了把“缓存稳定性”和“实现复杂度”都控制在可验证范围内，避免二阶段策略在不同 turn 中漂移。

### D4: persisted reference 仍通过 `readFile("tool-results/...")` 回读，不新增第二套恢复读取协议

Astrcode 已经允许 `readFile` 从 session 目录下解析 `tool-results/**`。  
因此第一阶段明确约定：

- 模型看到的完整 persisted reference 就是 `<persisted-output>`
- 若需要全量内容，继续用 `readFile("tool-results/<id>.txt")` 回读

这样有三个好处：

- 不新增“读取持久化工具结果”的专用协议
- 保持工具系统边界清晰，LLM 仍然只通过工具访问文件内容
- 与当前 `adapter-tools` 安全边界、路径沙箱实现一致

### D5: 第一阶段不做 prompt-local feedback package，summary 退到第二阶段

此前设想的 `ToolFeedbackPackage` 容易把“UI 摘要”“prompt 摘要”“durable truth”混在一起。  
基于 Claude Code 的真实实现，本次明确改成：

- 第一阶段主机制是 persisted reference + reread
- 若后续确实发现某些工具结果“适合本地 synthesis 而不适合 reread”，再单开第二阶段 change
- `tool_use_summary` 类能力若存在，也只服务 UI / diagnostics，不定义 prompt truth

## Risks / Trade-offs

- [Risk] aggregate budget 与 per-tool inline limit 叠加后，决策顺序变复杂
  - Mitigation：固定执行顺序为“aggregate budget 在前，其他 compaction 在后”，并用测试锁定

- [Risk] replacement decision 若未 durable 化，resume 后会出现不同 prompt 前缀
  - Mitigation：把 `replacement` 全文本写入 `ToolResultReferenceApplied` 事件，不在恢复时重新生成

- [Risk] 过多结果被外置后，模型反而频繁需要 reread
  - Mitigation：第一阶段只在 over-budget message 上替换 largest fresh results，并通过 observability 记录 reread 压力

- [Risk] `<persisted-output>` 再次进入后续 compaction 流程时被误处理
  - Mitigation：将“已是 persisted reference”视为已 compacted 内容，aggregate budget / prune / micro_compact 都必须显式跳过

- [Risk] 事件模型变更影响 projection / replay
  - Mitigation：把新事件限定为 session-runtime 内部事实，不改协议 DTO；同步补 projection / recovery 测试

## Migration Plan

1. 在 `core` 中补充 replacement event 类型与相关恢复状态模型
2. 在 `session-runtime/turn/request` 中加入 aggregate tool-result budget 入口
3. 在 projection / recovery 链路中重建 `ToolResultReplacementState`
4. 为 turn summary / observability 增加 replacement 命中指标
5. 对 `grep` / `shell` / `readFile` / resume 场景补全回归测试

回滚策略：

- 如果 aggregate replacement 效果不理想，可先关闭 request 侧 aggregate budget，仅保留现有 per-tool persisted-output 路径
- 新增 durable event 可继续保留，不要求立刻删除历史事件

## Open Questions

- 无。第一阶段采用 `<persisted-output> + aggregate budget + durable replacement state`，不再把 summary 作为主机制。
