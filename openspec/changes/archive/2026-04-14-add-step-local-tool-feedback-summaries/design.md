## Context

现在 turn 内的工具反馈链路大致是：

1. `tool_cycle` 产出 `raw_results` 和 `tool_messages`
2. `runner/step` 把 `tool_messages` 直接追加进 `execution.messages`
3. 下一轮 `turn/request` 再经过 `micro_compact`、`prune_pass`、`auto_compact`

这种方式的优点是简单、事实完整；缺点是“对模型最有用的反馈”没有被单独建模。尤其当只读工具很多时，下一轮往往会面对：

- 一堆原始输出
- 一堆被清理后的占位文本
- 少量 file recovery 补充

却没有一段稳定的“这批工具到底发现了什么”的摘要输入。

## Goals / Non-Goals

**Goals:**

- 在 tool cycle 与下一轮 request 之间增加 step-local 的工具反馈打包阶段
- 保持原始 tool result 仍然是 durable 事实源
- 让反馈打包遵守现有 budget / prune / compact 边界
- 让该能力可度量、可关闭、可回退

**Non-Goals:**

- 不让反馈打包替代原始 event log
- 不在这次 change 里引入新的全局 summarizer 服务
- 不把 request assembly 的职责重新塞回 `context_window`
- 不要求所有工具都必须有专门的定制摘要器

## Decisions

### D1: 引入 prompt-local `ToolFeedbackPackage`，但不改变 durable 真相

本次 change 倾向在 `session-runtime/turn` 或相邻 request 子域中引入 `ToolFeedbackPackage` 之类的内部结构，用于表达：

- 覆盖了哪些 `tool_call_id`
- 采用了什么打包策略
- 打包后的文本或结构化提示内容

它只服务于 turn 内下一轮 prompt 消费；原始 `ToolResult` 事件和 `LlmMessage::Tool` 仍然保留，用于 replay、恢复和调试。  
这与 Claude Code 里的异步 `toolUseSummary` 不同：Astrcode 第一阶段要解决的是“给下一轮模型什么反馈最有用”，不是“给 UI 生成一句短标题”。

### D2: 第一版优先使用本地、确定性的打包策略

为了不把 tool feedback packaging 变成第二条昂贵 LLM 主路径，第一版优先采用本地规则：

- 汇总工具名、目标对象和关键元数据
- 对 clearable/read-only 工具抽出“已检查什么、命中什么、下一步该看什么”
- 对超长输出保留少量关键片段而不是整段原文

后续如果需要更强摘要能力，可以在这套结构上追加异步 summarizer，但不作为第一版前置条件。

### D3: request assembly 消费反馈包时仍受 budget 约束

`turn/request` 负责最终 prompt 组装，因此它继续拥有“消费什么、舍弃什么”的最终权力。  
工具反馈包不是绕过 request assembly 的快捷通道，而是它的新输入之一：

- 优先消费反馈包
- 再决定保留多少原始 tool result
- 必要时再进入 prune / compact / recovery

这保持了当前 `context_window` 与 `turn/request` 的职责分离。

### D4: prompt-local 反馈包与异步 UI 摘要分层设计

如果后续需要类似 Claude Code 的异步短摘要能力，它应被视为独立的第二层：

- `ToolFeedbackPackage` 负责下一轮 prompt 消费
- UI/diagnostic summary 负责展示和检索便利性

第一阶段只落地前者，不让 UI 文案生成路径反向定义 prompt 语义。

### D5: feedback packaging 必须可观测

如果没有命中率和效果诊断，反馈包很容易变成另一种“看起来优雅但并不降低上下文噪音”的抽象。  
因此需要至少记录：

- 多少 step 生成了反馈包
- 覆盖了多少 tool calls
- 替代了多少原始字节或消息
- 因 budget 或策略未命中的原因

## Risks / Trade-offs

- [Risk] 本地规则摘要过于粗糙，反而遗漏重要细节
  - Mitigation：第一版只替代 clearable/read-heavy 场景，并保留回退到原始结果的能力

- [Risk] 新增反馈包后，request assembly 逻辑再次变重
  - Mitigation：把反馈包建模成明确输入，保持 `turn/request` 只做选择与组装，不承担摘要生成

- [Risk] 原始 tool result 与反馈包并存，可能造成提示词重复
  - Mitigation：为反馈包记录覆盖的 `tool_call_id`，由 request assembly 去重和裁剪

- [Risk] 团队把反馈包误当成“新的 durable 真相”继续向外扩散
  - Mitigation：在设计与命名上明确它是 prompt-local package，不进入事件主事实流

## Migration Plan

1. 新增工具反馈打包模块与内部数据结构
2. 在 `runner/step` 的 tool cycle 之后生成 step-local feedback package
3. 让 `turn/request` 学会消费 feedback package，并按 budget 去重/裁剪
4. 为命中率、覆盖范围和节省量增加 observability
5. 按工具类型逐步扩大适用面

回滚策略：

- 保留 feedback package 生成逻辑，但关闭 request 侧消费，系统即可退回当前原始结果路径

## Open Questions

- `ToolFeedbackPackage` 更适合做成内部结构体，还是需要一个明确的 durable 事件/投影协议来支持更强的调试与回放？
- 第二阶段是否需要引入“异步 LLM 工具摘要器”，还是本地规则已足够覆盖主要收益？
