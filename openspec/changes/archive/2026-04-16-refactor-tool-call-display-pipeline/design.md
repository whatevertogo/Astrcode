## Context

当前工具调用展示链路跨越 `session-runtime`、`application`、`server`、`protocol` 和 `frontend` 五层，但职责边界没有收稳：

- `session-runtime` 目前主要暴露 `SessionTranscriptSnapshot` 与 `SessionReplay`，仍然是 `records + replay/live receiver` 级别的原始事实。
- `application` 在 conversation/terminal 路径上主要做编排与透传，没有把工具展示收敛成稳定读模型。
- `server` 中的 `terminal_projection` 实际承担了状态型 read model 拼装责任，包括 tool call / tool stream / assistant block 的聚合与增量对齐。
- `protocol` 里的 `conversation v1` 仍然直接复用 `terminal v1` DTO，工具展示所需的 `error`、`durationMs`、`truncated` 等字段没有成为稳定合同的一部分。
- `frontend` 拿到 block 后还要继续做本地投影、相邻分组和 metadata 补偿，例如把相邻的 `tool_stream` 归并到 `tool_call`，再从 `metadata` 中推断 sub-run / child session 关联。

这违背了仓库当前的核心约束：

- `server` 应该是唯一业务事实的传输边界，而不是新的状态真相面。
- `application` 应该暴露稳定用例结果，而不是让前端继续消费底层 transcript/replay 细节。
- `protocol` 应保持纯 DTO，不承载运行时逻辑。
- `frontend` 只能消费 authoritative HTTP / SSE 合同，不应维护工具展示的平行语义。

这次变更是典型的跨层改动：会影响读模型建模、协议合同、SSE delta 形状、前端渲染路径以及 reconnect/replay 行为，因此需要先固定技术设计，再落到 specs 和实现。

## Goals / Non-Goals

**Goals:**

- 把工具调用展示链路收敛成以后端为唯一真相源的 authoritative read model。
- 让 `session-runtime` 查询侧直接产出 conversation/tool display 所需的稳定结构，而不是只暴露原始 event/replay 事实。
- 让 `application` 暴露稳定的 conversation/tool display facts，`server` 只负责薄投影和 SSE framing。
- 定义稳定的工具展示 DTO，显式覆盖工具终态、错误、耗时、截断标记、stdout/stderr 和子会话关联。
- 删除前端对 tool stream 的相邻 regroup、tool metadata 猜测和脆弱 fallback 匹配。
- 保持 live 增量、durable replay、cursor catch-up 和 rehydrate 语义一致，不因为重构引入第二套恢复机制。
- 让实现顺序本身可验证：每一阶段都必须有明确测试与验收出口，再进入下一阶段。

**Non-Goals:**

- 不在本次变更中重做所有 transcript block 的视觉呈现或交互样式。
- 不引入新的传输协议或额外 server 组合根，仍然基于现有 HTTP / SSE 和 `astrcode-server`。
- 不改变工具执行本身、tool scheduling 策略或 LLM/tool orchestration 主流程。
- 不在本次变更中统一重构所有 child/sub-run 展示，只处理它与 tool call 展示的契约边界。
- 不追求对旧 conversation/terminal 工具展示 DTO 的长期兼容；兼容逻辑只作为迁移阶段临时措施存在。

## Decisions

### 决策 1：将工具展示 authoritative read model 下沉到 `session-runtime/query`

新增 `session-runtime` 内部的 conversation/tool display 查询子域，由它直接从 durable event log、projected state 和 live 事件恢复出“工具展示所需的稳定语义”，而不是让 `server` 内的 projector 继续承担状态型聚合。

具体做法：

- `session-runtime/query` 新增面向 conversation surface 的读模型类型，例如：
  - `ConversationSnapshotFacts`
  - `ConversationReplayFacts`
  - `ConversationBlockFacts`
  - `ToolCallBlockFacts`
- 读模型内部负责把 `ToolCall`、`ToolCallDelta`、`ToolResult`、相关 child notification / collaboration 事实聚合到稳定的 tool display 实体。
- 聚合必须同时支持三种来源：
  - durable replay
  - live-only delta
  - replay + live catch-up 之间的去重/对齐

这样做的原因：

- `session-runtime` 才是单 session 真相面；工具展示的聚合语义本质上属于单 session query，而不是 HTTP 投影。
- 当前 `server/http/terminal_projection.rs` 持有的 block 聚合状态已经越过“薄映射层”边界，长期会继续膨胀。
- 把聚合下沉后，Web/Desktop/TUI 都可以共享同一份 authoritative conversation/tool read model，而不是每个 surface 再做一层私有二次解释。

备选方案：

- 继续把聚合逻辑留在 `server` projector 中，只补协议字段。
  - 放弃原因：这只能缓解字段缺失，不能解决真相层级错误和前端二次拼装问题。
- 把聚合逻辑放进 `application`。
  - 放弃原因：`application` 应负责用例编排，不应持有单 session replay/live 聚合细节。

### 决策 2：`application` 改为返回 conversation/tool display facts，而不是 transcript/replay 原始事实

`application` 在 conversation/terminal 用例路径上不再直接暴露 `SessionTranscriptSnapshot` 和 `SessionReplay` 作为上层构建材料，而是调用 `session-runtime` 新的 query 接口，返回已经过边界收敛的稳定 facts。

职责调整如下：

- `session-runtime`
  - 持有事件日志、回放、live receiver、聚合恢复和 tool display read model
- `application`
  - 校验 session/focus/cursor
  - 编排 control state、child summaries、slash candidates
  - 返回 conversation/tool display facts
- `server`
  - 将 facts 映射为 DTO
  - 负责 HTTP 状态码、SSE envelope、auth 和 rehydrate framing

这样做的原因：

- 这符合 `application-use-cases` 和 `PROJECT_ARCHITECTURE.md` 中“应用层暴露稳定用例结果、单 session 真相由 session-runtime 持有”的边界。
- 这可以让 `server` 停止接触 `SessionTranscriptSnapshot.records` 和原始 replay receiver，避免传输层继续长出业务态。

备选方案：

- 让 `application` 继续透传 transcript/replay，前端少量调整。
  - 放弃原因：这仍然要求上层理解底层事件拼装规则，违背 authoritative read model 的目标。

### 决策 3：工具展示协议改为“聚合 tool block + 受控 stream patch”模型

协议层不再把工具展示建模成“一个 `tool_call` block + 若干前端需要相邻 regroup 的 sibling `tool_stream` blocks”。新的合同改为：

- `tool_call` 是唯一的工具展示主实体，包含：
  - 稳定 `toolCallId`
  - `toolName`
  - `status`
  - `input`
  - `summary`
  - `error`
  - `durationMs`
  - `truncated`
  - `metadata`
  - `childRef`
  - `streams`（按 `stdout` / `stderr` 分槽，作为 tool block 的组成部分）
- live 增量对工具输出的更新以“patch 某个 tool call block 的某个 stream channel”为单位，而不是追加独立 transcript sibling block。
- replay/hydration 直接返回完整 tool block；stream 阶段只发送该 block 的 patch。

这样做的原因：

- 当前前端的 `groupedToolStreams` 依赖消息相邻顺序，这对并发工具、交错 stdout/stderr 和 future nested tool UI 都很脆。
- `ToolExecutionResult` 已经天然是聚合结果，协议应该尊重这个聚合模型，而不是把最终语义拆碎后再要求前端拼回来。
- stream channel 仍然是纯 DTO 字段，不需要在协议层承载运行时逻辑。

备选方案：

- 保留独立 `tool_stream` block，但给它增加更多父子字段。
  - 放弃原因：前端仍需要维护 grouping/rendering 规则，contract 仍然脆弱。
- 彻底只保留最终 `ToolExecutionResult`，不再暴露 stdout/stderr 增量。
  - 放弃原因：会损失工具 streaming 体验，与现有产品面要求不符。

### 决策 4：`conversation` 在现有 `v1` 内直接收口为独立合同

`conversation` 不再长期通过 `pub use terminal::v1::*` 复用全部 DTO，而是演进为独立的协议命名空间，由它承载面向多 surface 的 authoritative conversation read contract。

演进策略：

- 直接修改当前 `conversation v1` 的实际定义，不再通过 `pub use terminal::v1::*` 维持假性独立。
- `terminal` 可以在迁移期间复用 conversation 的内部 mapper 或过渡 DTO，但 conversation 是唯一 authoritative contract。
- 由于仓库当前不维护向后兼容，本次不额外引入 `conversation v2` 作为过渡版本。

这样做的原因：

- 当前问题首先出在 conversation surface 本身；继续复用 terminal alias 会把临时结构固化成长期设计。
- 独立命名空间更符合“protocol 只定义稳定 wire contract”的原则，后续也更方便进行破坏性演进。

备选方案：

- 继续保留别名，只在注释层声明 conversation 是 authoritative。
  - 放弃原因：无法真正收拢 contract，也无法避免后续 surface 再次互相绑死。

### 决策 5：前端只维护 block store 与渲染，不再重建工具展示语义

前端 conversation 路径改为直接消费 authoritative tool block：

- `conversation.ts` 只负责：
  - 解析 snapshot / stream envelope
  - 应用 block upsert / patch
  - 维护 cursor 与当前 block store
- `ToolCallBlock` 直接读取完整 tool block 渲染，不再依赖外部聚合后的 `groupedToolStreams`
- `MessageList` 删除对相邻 `toolStream` 的扫描分组逻辑
- `subRunView` 不再从 `spawn` tool metadata 猜测拓扑作为主路径；tool 与 child 的关联必须来自显式字段

这样做的原因：

- 前端最适合做视图层 patch 和渲染，不适合继续持有 conversation semantics。
- 这能显著减少 reconnect/catch-up、late metadata patch、并发工具交错输出时的状态错位。

备选方案：

- 保留前端 regroup，但让后端保证更强的排序。
  - 放弃原因：排序约束很难覆盖所有 future case，而且仍然把语义责任留在前端。

### 决策 6：迁移采用“测试先行 + 分层切换 + 最小过渡层”的方式收敛风险

这次改动涉及协议和前端消费路径，不能直接大爆炸替换。迁移按以下顺序进行：

1. 先补 fixture 和跨层行为测试，冻结当前正确行为与目标行为。
2. 在 `session-runtime` 实现新的 conversation/tool display facts。
3. 让 `application` / `server` 改为消费这些 facts，并引入最小过渡映射。
4. 前端切换到新的 tool block contract。
5. 删除旧的 projector / regroup / fallback。

这样做的原因：

- 当前问题最怕的是“改了很多层，但没有稳定行为基线”，因此必须先锁回归测试。
- 双轨阶段可以让我们逐层验证 replay/live/catch-up 行为，而不是一次性切断。

备选方案：

- 直接全量切换，依赖手工验证。
  - 放弃原因：风险过高，且很难在并发工具/重连场景下保证不回归。

## Target Model

### 目标读取链路

```text
durable event log + projected state + live event
    -> session-runtime/query conversation read model
    -> application use case facts
    -> server dto mapping + sse framing
    -> frontend block store
    -> ui render
```

### 目标工具展示模型

每个 tool call 在读模型中只对应一个稳定实体：

- `toolCallId`
- `turnId`
- `toolName`
- `status`
- `input`
- `summary`
- `error`
- `durationMs`
- `truncated`
- `streams.stdout`
- `streams.stderr`
- `childRef`
- `metadata`

约束：

- hydration 返回完整 tool block
- live 更新只 patch 该 tool block
- replay/catch-up 与 live 去重必须在同一聚合层完成
- 客户端不得通过相邻 block 顺序、toolName 特例或 metadata fallback 重建主语义

## Risks / Trade-offs

- [Risk] `session-runtime/query` 的 conversation 聚合复杂度上升，可能把查询层做成新的巨石  
  → Mitigation：只下沉单 session authoritative read model，不把 surface-specific 样式、HTTP framing 或前端 patch 细节放进去。

- [Risk] conversation DTO 拆离 `terminal` 后会产生短期重复结构  
  → Mitigation：允许实现期共享内部 mapper/辅助结构，但对外 contract 必须先解耦，再逐步消除重复。

- [Risk] live-only delta 与 durable replay 对齐不当，会出现 tool stream 重复或缺块  
  → Mitigation：把 replay/live 去重规则收敛到同一个 read model，并用 fixture 覆盖“live 先到、durable 后补”的场景。

- [Risk] 前端切换新 contract 时，sub-run / child session 视图可能受 tool metadata 路径变化影响  
  → Mitigation：在迁移阶段保留旧字段映射，同时优先补充显式关联字段，再删除 metadata 猜测逻辑。

- [Risk] 破坏性 DTO 变更会影响现有 terminal/web surface  
  → Mitigation：以 conversation contract 为主线推进，保留一小段兼容层，待前端全部切换后再统一移除。

- [Trade-off] 这次会增加短期重构成本，尤其是 `session-runtime` 查询侧和前端 block store 的调整  
  → 换来的收益是长期边界收敛：后端持有真相、协议可演进、前端逻辑显著变薄。

## Migration Plan

1. 为当前工具展示行为补充 fixture 与跨层测试，覆盖：
   - 单工具 stdout/stderr streaming
   - 并发工具交错输出
   - failed tool 的错误展示
   - duration/truncated 字段可见
   - late metadata / child session 关联
   - replay/catch-up/rehydrate 恢复
2. 在 `session-runtime/query` 引入新的 conversation/tool display facts，并让 `application` 改为消费这些 facts。
3. 在 `protocol` 中定义新的 conversation tool contract，并在 `server` 中同时提供兼容映射。
4. 前端切换到新 contract，删除 `groupedToolStreams` 和相邻 regroup。
5. 清理 `server` 中不再需要的状态型 projector 逻辑，收回到薄映射层。
6. 移除兼容 DTO 和旧前端 fallback，完成收口。

回滚策略：

- 若新 contract 在前端展示上出现严重回归，优先回退前端消费路径到旧 contract，同时保留后端新 query 实现。
- 若问题出在后端聚合规则，可暂时恢复旧 server projector 输出，但保留新测试和新 contract 草图，避免丢失定位成果。
- 回滚边界以“contract 层”和“消费层”分离，避免整个 change 只能整体回退。

## Acceptance Gates

### Gate 1：基线冻结完成

- 缺失场景测试已补齐
- replay/live/catch-up/rehydrate 有固定回归
- 协议 fixture 能稳定比较 hydration 与 delta 形状

### Gate 2：后端 authoritative read model 完成

- `session-runtime/query` 已输出 conversation/tool display facts
- `application` 不再向上传 transcript/replay 原始事实
- `server` 不再持有状态型 tool 聚合真相

### Gate 3：前端消费路径切换完成

- conversation block store 直接消费 authoritative tool block
- `MessageList` 不再 regroup sibling streams
- `subRunView` 不再依赖 `spawn` metadata 猜测主拓扑

### Gate 4：收尾完成

- 旧兼容层已删除
- 相关文档已同步
- 验证命令与手动验收已完成
