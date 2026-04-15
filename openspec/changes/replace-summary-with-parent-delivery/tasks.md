## 1. Contract / Event Model

- [x] 1.1 在 `core` 中稳定 typed parent-delivery message 与 delivery kind enum；`payload` 必须按 `kind` 做判别联合并为 `completed`/`failed`/`close_request`/`progress` 定义最小字段集，至少包含 `kind`、`payload`、`terminal semantics`、`idempotency key`、`origin(explicit|fallback)` 与 `sourceTurnId`。
- [x] 1.2 盘点并标记被替代的旧合同：`ChildSessionNotification.summary`、`SubRunHandoff.summary`、`final_reply_excerpt` 与 server summary projection；先新增 typed DTO 与 replay upgrade 读路径，再删除旧字段，不能把“引入”和“删除”放在同一步。
- [x] 1.3 为 `protocol` / `server` 增加 typed upward delivery 的传输 DTO 与 durable carrier event 映射，并更新 collaboration / diagnostics fact；首批父视图主流程只依赖 P0/P1，P2 先保证 durable contract、路由与序列化正确。

## 2. Unified Send Routing

- [x] 2.1 在 `application` 与 `adapter-tools` 中把 `send` 改造成方向感知的统一业务入口：`-> direct child` 保持具体指令语义，`-> direct parent` 发送 typed upward delivery；中间层 agent 在同一 turn 内可同时上行和下行 `send`，禁止新增独立 `reply_to_parent` 工具。
- [x] 2.2 锁死 unified `send` 的 routing invariant、ownership verification 与非法跨树调用错误：child 不能伪造 parent routing context，root 不能冒充 child 上行，非法调用必须在进入 wake/finalizer 前前置拒绝。
- [x] 2.3 固定 unreachable-parent policy：parent 缺失、已关闭或当前不可达时，统一在 `application` 层前置拒绝并打结构化 log / collaboration fact，禁止在 handler、mapper 或前端投影层各自临时兜底。
- [x] 2.4 明确 unified `send` 的幂等与重放合同：重试 / replay 不得制造重复 terminal delivery，typed upward delivery 的幂等键必须稳定复用。

## 3. Delivery Execution

- [x] 3.1 修改 `crates/application/src/agent/terminal.rs`、`wake.rs` 与相关路由链路，让 child 通过 unified `send` 显式上行成为主路径；finalizer 必须基于当前 `turn_id` 对应的 child work turn 是否已有 terminal upward delivery 判定是否需要 fallback。
- [x] 3.2 定义 deterministic fallback 的触发与内容来源：每个 child work turn 最多一次，只能来自最终 assistant message / terminal reason 等 deterministic fact，并写入 durable event log；fallback 不得冒充显式上行，必须带 `origin=fallback` 等来源标记。
- [x] 3.3 在 `crates/server/src/http/mapper.rs` 与相关 HTTP 合同测试中切换到 typed upward delivery；server 只做 contract 映射与转发，不再生成 summary。
- [x] 3.4 等后端与前端消费者全部切换完成后，再删除 child notification、subrun handoff 与 server DTO 中旧的 summary 主字段及相关 synthetic summary event；删除新写路径不等于删除历史旧事件的 replay / upgrade 读能力。

## 4. Prompt / Governance

- [x] 4.1 更新 child-scoped prompt contract 与 governance guidance，要求 agent 在需要委派下游时通过 unified `send` 向 direct child 发具体指令，在完成、失败或请求结束分支时通过 unified `send` 向 direct parent 发正式消息，并明确中间层 agent 可同时使用两种方向。
- [x] 4.2 明确 prompt 只是提高显式上报命中率；系统 correctness 仍由 business contract、ownership verification、前置拒绝与 terminal fallback 保证，禁止依赖额外一轮“是否完成”的 LLM 追问。
- [x] 4.3 删除旧的 summary-oriented child contract 文案、独立 `reply_to_parent` 文案与相关测试夹具。

## 5. Frontend Projection

- [x] 5.1 在 `frontend/src/types.ts` 与 `lib/agentEvent.ts` 中新增 typed upward delivery 事件类型与解码，不再把 summary 作为父会话主合同。
- [x] 5.2 更新 `lib/applyAgentEvent.ts` 与 `lib/subRunView.ts`，让父会话投影消费 typed upward delivery、close request 与 fallback delivery，并保持独立子会话入口可达。
- [x] 5.3 调整 `frontend/src/components/Chat/*` 的父视图展示，移除 summary card 依赖；同时显式采用历史 session 的 mapper-upgrade replay 策略，不做旧 live contract 长期兼容。

## 6. Validation / Replay

- [x] 6.1 为 unified `send` 补单元与集成测试，基础覆盖 direct-parent ownership、非法跨树调用、前置拒绝日志、P0/P1 terminal kind、以及显式 terminal send 后不再 fallback；P2 至少覆盖合同序列化与路由正确性，不要求首批完整 UI 语义。
- [x] 6.2 补后端测试：terminal fallback、wake requeue、event replay / session recovery 后不重复投递、parent 重载后 typed upward delivery 投影一致。
- [x] 6.3 补前端测试：父视图不再依赖 summary、子会话入口仍可打开、fallback delivery 正常展示、旧 event replay 不会把父视图投影坏掉。
- [x] 6.4 运行 `cargo fmt --all`、相关 Rust 测试，以及 `cd frontend && npm run typecheck && npm run test -- ...`。
