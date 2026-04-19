## 0. 执行原则

- [x] 0.1 任何结构性重构开始前，先补测试与 fixture；没有行为基线，不进入 `session-runtime` / protocol / frontend 主改动。
- [x] 0.2 每完成一个阶段，就同步更新本文件的 checkbox，并记录是否达到该阶段的验收出口。

## 1. 冻结工具展示行为基线

### 阶段目标

把当前工具展示链路的真实期望行为固定下来，覆盖 snapshot、live、replay、catch-up 和 reconnect，避免后续重构把问题从一层挪到另一层。

- [x] 1.1 盘点当前工具展示链路涉及的测试与 fixture，补齐 `crates/server/src/http/terminal_projection.rs`、`frontend/src/lib/api/conversation.test.ts`、`frontend/src/components/Chat/ToolCallBlock.test.tsx` 中缺失的并发工具、交错 stdout/stderr、failed tool、late metadata / late child association 场景。
- [x] 1.2 为 replay/live 对齐补充后端回归测试，覆盖 `session-runtime` live 先到、durable 后补、cursor catch-up、rehydrate 恢复等行为，优先落在 `crates/session-runtime/src/turn/tool_cycle.rs`、`crates/server/src/http/routes/conversation.rs` 和相关测试文件。
- [x] 1.3 固化 conversation/tool display 的 JSON fixture 或快照，至少锁定 hydration、append delta、tool patch、rehydrate envelope 的输出形状。

### 阶段验收

- [x] 1.4 确认第 1 阶段新增测试在不改生产逻辑的前提下能够表达目标行为，并作为后续重构的回归护栏。

## 2. 下沉 authoritative read model 到 session-runtime

### 阶段目标

把 conversation/tool display 的真相聚合从 `server` projector 下沉到 `session-runtime/query`，让单 session 读取语义回到运行时查询边界。

- [x] 2.1 在 `crates/session-runtime/src/query/` 下新增 conversation/tool display 查询子域，定义稳定读取类型，例如 `ConversationSnapshotFacts`、`ConversationReplayFacts`、`ConversationBlockFacts`、`ToolCallBlockFacts`，并调整 `query/mod.rs` 导出。
- [x] 2.2 实现基于 durable event log、projected state 和 live 事件的 tool display 聚合逻辑，覆盖 `ToolCallStart`、`ToolCallDelta`、`ToolCallResult`、child notification、late metadata 与 childRef 关联。
- [x] 2.3 把 replay/live 去重、stream 归属与 tool block patch 生成规则收敛到同一聚合层，不再让 `server` 或前端重复解释这些规则。
- [x] 2.4 为新的 query 子域补齐单元测试和集成测试，明确它只负责单 session read model，不承载 HTTP/SSE framing、surface 样式或前端补丁策略。

### 阶段验收

- [x] 2.5 确认 `session-runtime/query` 已能独立输出 authoritative conversation/tool display facts，且相关测试覆盖 replay/live/catch-up 的核心路径。

## 3. 收口 application、server 与 protocol 合同

### 阶段目标

让 `application` 只编排稳定 facts，让 `server` 退回薄映射层，同时把 conversation/tool display wire contract 收紧成可长期演进的协议。

- [x] 3.1 调整 `crates/application/src/terminal_use_cases.rs` 与 `crates/application/src/terminal/mod.rs`，让 conversation/terminal 用例改为消费 `SessionRuntime` 新的 query 接口，而不是继续向上传 `SessionTranscriptSnapshot`、`SessionReplay` 或原始 receiver。
- [x] 3.2 重构 `crates/server/src/http/routes/conversation.rs` 与 `crates/server/src/http/terminal_projection.rs`，把 `server` 收回到 DTO 映射、HTTP 状态码和 SSE framing；删除或显著收缩状态型 tool 聚合逻辑。
- [x] 3.3 在 `crates/protocol/src/http/conversation/` 中定义独立的 conversation contract，不再通过 `pub use terminal::v1::*` 维持别名关系。
- [x] 3.4 在必要时调整 `crates/protocol/src/http/terminal/`，把工具展示 DTO/patch 形状收敛为单个 tool block + stream patch 模型，显式覆盖 `error`、`durationMs`、`truncated`、`streams` 与 `childRef` / sub-run 关联。
- [x] 3.5 补齐协议序列化测试与 fixture，确保 snapshot、delta、rehydrate、tool patch 的 wire shape 可稳定比较。
- [x] 3.6 仅保留最小必要的迁移兼容层；兼容逻辑不得重新把业务聚合塞回 DTO 层或 `server` route。

### 阶段验收

- [x] 3.7 确认 `application` 不再向上传 transcript/replay 原始事实，`server` 不再持有 authoritative tool aggregation，conversation contract 已独立成形。

## 4. 切换前端到 authoritative tool block

### 阶段目标

前端不再重建工具语义，而是直接消费后端 authoritative tool block，保留最小 block store 和渲染职责。

- [x] 4.1 重构 `frontend/src/lib/api/conversation.ts`，让 snapshot/envelope 直接维护 authoritative tool block store，删除相邻 `tool_stream` regroup、tool metadata fallback 和依赖旧 block 语义的本地重建逻辑。
- [x] 4.2 调整 `frontend/src/components/Chat/MessageList.tsx`，删除 `groupedToolStreams` 和 sibling stream 扫描逻辑；tool UI 只接收单个完整 tool block。
- [x] 4.3 调整 `frontend/src/components/Chat/ToolCallBlock.tsx` 与相关类型定义，让它直接渲染完整 tool block 中的 streams、终态字段和 child 关联，不再依赖外部聚合后的 stream 列表。
- [x] 4.4 收敛 `frontend/src/lib/subRunView.ts` 中对 tool metadata 的拓扑猜测路径，优先消费新的显式 child/sub-run 关联字段，而不是依赖 `spawn` tool metadata 推断主语义。
- [x] 4.5 更新前端回归测试，覆盖 hydration、stream patch、并发工具交错输出、failed / truncated、late child session 关联和 reconnect 后的渲染稳定性。

### 阶段验收

- [x] 4.6 确认前端已不再依赖 `tool_stream` 相邻顺序、metadata fallback 或 `spawn` 特判来恢复工具展示真相。

## 5. 清理、文档与验证

### 阶段目标

删除过渡逻辑，完成文档同步和验证，确保新边界真正落地，而不是停留在“同时保留两套实现”的状态。

- [x] 5.1 删除迁移完成后不再需要的旧 projector / fallback / regroup 逻辑，清理无效 DTO 字段和前端兼容分支，确保职责边界与命名保持清晰一致。
- [x] 5.2 同步更新必要文档，至少核对 `PROJECT_ARCHITECTURE.md` 中 `application`、`session-runtime`、`server`、`protocol` 边界描述，以及 conversation/tool display contract 相关说明。
- [x] 5.3 运行并通过本次改动涉及的后端与前端验证命令；至少覆盖相关 Rust 测试、`cargo fmt --all`、前端 `npm run typecheck`、`npm run lint` 与 conversation/tool display 相关测试。
- [x] 5.4 做一次端到端手动验收：验证工具调用在 snapshot、live streaming、并发交错、失败、截断、子会话关联、cursor catch-up、reconnect / rehydrate 场景下都稳定展示且不再依赖本地二次拼装。
- [x] 5.5 在所有阶段验收通过后，再考虑 sync main specs 与 archive；未通过前不得把该 change 标记为实现完成。
