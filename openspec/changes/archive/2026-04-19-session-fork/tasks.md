## 1. 类型定义与核心逻辑重构

- [x] 1.1 在 `crates/session-runtime/src/turn/` 下新增 `fork.rs`，定义 `ForkPoint` 枚举（`StorageSeq(u64)` | `TurnEnd(String)` | `Latest`）和 `ForkResult` 结构体（`new_session_id`, `fork_point_storage_seq`, `events_copied`），在 `turn/mod.rs` 中导出
- [x] 1.2 在 `crates/session-runtime/src/turn/branch.rs` 中，将现有 `branch_session_from_busy_turn` 的事件复制逻辑（replay → 按 seq 截断 → 创建新 session → 逐条 append → 广播 catalog event）提取为内部方法 `fork_events_up_to`，现有 `branch_session_from_busy_turn` 改为调用它，行为不变
- [x] 1.3 `cargo test -p astrcode-session-runtime` — 现有测试全部通过，确保重构未引入回归

## 2. fork_session 实现

- [x] 2.1 在 `crates/session-runtime/src/turn/fork.rs` 中实现 `pub async fn fork_session(&self, source_session_id: &SessionId, fork_point: ForkPoint) -> Result<ForkResult>`，内部逻辑：replay 源 session → 按 `ForkPoint` 解析目标 `storage_seq`（`TurnEnd` 找 `TurnDone` 的 seq；`Latest` 找稳定前缀末尾）→ 稳定前缀校验（目标 seq 之后不能有未完成 turn 的事件）→ 调用 `fork_events_up_to` → 返回 `ForkResult`
- [x] 2.2 编写单元测试覆盖：尾部 fork（源 Idle → 全量复制）、尾部 fork（源 Thinking → 截到稳定前缀）、`StorageSeq` 稳定点 fork、`TurnEnd` 已完成 turn fork、未完成 turn → Validation、活跃 turn 内 seq → Validation、不存在 turn_id → NotFound、新 session `SessionStart` 谱系字段正确、新 session 投影 `phase = Idle`
- [x] 2.3 `cargo test -p astrcode-session-runtime` 全部通过

## 3. Application 层 use case

- [x] 3.1 在 `crates/application/src/session_use_cases.rs` 新增 `pub async fn fork_session(&self, session_id: &str, fork_point: ForkPoint) -> Result<SessionMeta>`，校验源 session 存在后调用 `session_runtime.fork_session`
- [x] 3.2 `cargo check -p astrcode-application` 通过

## 4. Protocol 层 DTO

- [x] 4.1 在 `crates/protocol/src/http/session.rs` 新增 `ForkSessionRequest` DTO（`turn_id: Option<String>`, `storage_seq: Option<u64>`，`#[serde(rename_all = "camelCase")]`），响应复用现有 `SessionListItem`
- [x] 4.2 `cargo check -p astrcode-protocol` 通过

## 5. Server 层 HTTP 端点

- [x] 5.1 在 `crates/server/src/http/routes/sessions/mutation.rs` 新增 `fork_session` handler：解析 `ForkSessionRequest` → 互斥校验（`turn_id` 和 `storage_seq` 同时存在返回 400）→ 转为 `ForkPoint` → 调用 `app.fork_session` → 返回 `SessionListItem`
- [x] 5.2 在 `crates/server/src/http/routes/mod.rs` 注册路由 `.route("/api/sessions/{id}/fork", post(sessions::mutation::fork_session))`
- [x] 5.3 编写集成测试：无参数尾部 fork 成功、带 turnId 已完成 turn fork 成功、带 turnId 未完成 turn → 400、同时传 turnId 和 storageSeq → 400、不存在 session → 404、成功时 `parentSessionId` 指向源 session
- [x] 5.4 `cargo test -p astrcode-server` 全部通过

## 6. 前端对接

- [x] 6.1 在 `frontend/src/lib/api/sessions.ts` 新增 `forkSession(sessionId: string, options?: { turnId?: string; storageSeq?: number })` 函数
- [x] 6.2 在前端新增 fork 目标解析逻辑：消息级入口只作为客户端便捷映射，能从历史消息解析到所属已完成 turn 的 `turnId`；不能稳定映射的消息不显示入口
- [x] 6.3 在已完成 turn 的上下文菜单和可映射的历史消息入口中添加"从此处 fork"操作，成功后立即切换到新 session
- [x] 6.4 补充前端测试覆盖：可 fork 消息/turn 显示入口、不可映射消息不显示入口、fork 成功后切换到新 session
- [x] 6.5 `cd frontend && npm run typecheck` 通过，手动验收 fork 后切换到新 session 且历史对话正确
