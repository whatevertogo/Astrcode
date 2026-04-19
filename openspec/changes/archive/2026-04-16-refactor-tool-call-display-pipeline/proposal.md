## Why

当前工具调用展示链路把“真相”拆散在五层里：

- `session-runtime` 暴露的是 transcript / replay 级原始事实
- `application` 继续把这些原始事实往上传
- `server` 里的 projector 在临时承担状态型聚合
- `protocol` 没把工具展示需要的一等字段收成稳定合同
- `frontend` 还在依赖相邻 regroup、metadata fallback 和本地拓扑猜测来把 UI 拼回来

这导致了三个实际问题：

- 展示不稳定：并发工具、stdout/stderr 交错、failed tool、late child metadata 容易错位或丢字段
- 架构职责漂移：`server` 和 `frontend` 在补单 session 读模型真相，违反当前仓库边界
- 后续难演进：只要改 tool display contract，就会同时牵动 replay、SSE、前端分组和子会话拓扑推断

这次 change 的目标不是“修一个前端显示问题”，而是把工具调用从 durable/live event 到最终 UI block 的整条读取链路收口成以后端为真相源的稳定合同。

## What Changes

- 在 `session-runtime/query` 新增 conversation/tool display authoritative read model，直接输出工具展示所需的稳定 facts，而不是继续只暴露 transcript/replay 原始材料。
- 调整 `application` conversation/terminal 用例，让它只编排稳定 query result，不再把 `SessionTranscriptSnapshot`、`SessionReplay` 或 replay receiver 视为正式展示合同向上传递。
- 把 `server` 收回薄传输层，只保留 DTO 映射、HTTP 状态码、SSE framing 和 rehydrate signaling；tool 聚合、replay/live 去重和 block 归属不再留在 route/projector。
- 让 `conversation` contract 成为独立的 authoritative surface，工具展示以单个 tool block 为主实体，显式覆盖：
  - `toolCallId`
  - `toolName`
  - `status`
  - `input`
  - `summary`
  - `error`
  - `durationMs`
  - `truncated`
  - `streams`
  - `childRef` / sub-run 关联
- 重构前端 conversation 消费路径，删除：
  - 相邻 `tool_stream` regroup
  - metadata fallback 推断
  - 依赖 sibling block 顺序的 tool 渲染逻辑
- 用测试和 fixtures 冻结 snapshot / replay / live delta / reconnect / catch-up 行为，确保重构期间不会把旧问题换一种方式带回来。
- **BREAKING**：本次 change 会直接收紧现有工具展示 contract，不为旧的本地 regroup 语义提供长期向后兼容。

### 用户可见影响

- 工具调用展示会稳定许多：并发工具、交错 stdout/stderr、失败结果、截断结果和晚到 child 关联不应再错位。
- 工具详情会更完整：用户可以直接看到终态、错误、耗时、截断标记和子会话/子执行关联。
- reconnect / catch-up 后的会话恢复结果应与初始 snapshot 保持一致，不再依赖前端二次拼装补洞。

### 开发者可见影响

- 后端需要正式承担 tool display read model 的最终所有权，前端只保留 block store 与渲染。
- `session-runtime`、`application`、`server`、`protocol`、`frontend` 的 conversation/tool display 边界会被重新划清。
- 这次实现必须以测试先行推进；没有补齐基线测试、协议 fixture 和恢复路径回归前，不进入大规模重构。

### Non-Goals

- 本次不重做全部聊天 UI 视觉样式，也不扩展非工具类 block 的产品功能范围。
- 本次不引入新的传输协议（如 WebSocket），仍基于现有 HTTP / SSE 路径演进。
- 本次不改变 tool execution、tool scheduling、approval / governance 本身的业务语义。
- 本次不追求对旧工具展示 contract 的长期向后兼容；仓库当前优先干净架构和稳定新边界。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `terminal-chat-read-model`: 收紧 authoritative conversation read model 对 tool call / tool stream / tool result 的要求，明确工具聚合块、稳定标识、终态字段与客户端不得本地重组的约束。
- `terminal-chat-surface`: 调整终端/交互式 surface 对工具调用展示的可见行为要求，确保工具输出、失败态和子会话关联都来自同一服务端事实。
- `session-runtime-subdomain-boundaries`: 调整 `session-runtime` 查询侧边界，允许 conversation/tool display read model 下沉到合适的 query 子域，而不是长期停留在 `server` 投影层。
- `application-use-cases`: 调整 `application` 在 conversation/tool display 路径上的用例编排职责，要求其暴露稳定读模型事实而不是原始 transcript/replay 细节。

## Impact

- 影响代码：
  - `crates/session-runtime` 的 query / conversation read model
  - `crates/application` 的 terminal/conversation use case 编排
  - `crates/server` 的 conversation route 与 terminal projection
  - `crates/protocol` 的 conversation / terminal DTO
  - `frontend` 的 conversation API 投影、tool block 渲染、sub-run/thread patch 逻辑
- 影响系统：
  - live SSE 增量与 durable replay 的 tool 聚合行为
  - tool metadata 到 UI 的暴露合同
  - child/sub-run 与 tool call 的关联展示
- 实施顺序：
  1. 冻结当前行为基线和协议 fixture
  2. 下沉 `session-runtime` authoritative query
  3. 收紧 `application` / `server` / `protocol` 合同
  4. 切换前端消费路径
  5. 删除兼容层并完成验证
- 迁移思路：
  - 先把测试、fixtures 和恢复路径锁住
  - 再新增后端 authoritative tool contract，并保留最小必要的过渡映射
  - 然后切换前端消费到新 contract
  - 最后删除旧的 regroup / fallback / projector 过渡逻辑
- 回滚思路：
  - 若新 contract 导致前端展示回归，优先回退前端消费路径或 `server` 的兼容映射层
  - 在彻底删除旧逻辑前保留行为测试与 fixtures，确保可以按边界局部回退，而不是整体回滚
