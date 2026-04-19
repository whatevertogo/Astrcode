## 1. 核心模型与工具契约

- [x] 1.1 在 `crates/core/src/` 新增 task 稳定类型与 metadata schema 结构（`ExecutionTaskItem`、`ExecutionTaskStatus`、`TaskSnapshot`、owner 标识），并在 `crates/core/src/lib.rs` 导出；验证：`cargo test -p astrcode-core --lib`
- [x] 1.2 在 `crates/adapter-tools/src/builtin_tools/task_write.rs` 实现 `taskWrite` 输入 schema、校验规则（含 20 条上限、单 in_progress 约束）、prompt guidance（`ToolPromptMetadata`）、capability metadata（`SideEffect::Local`）和 `ToolExecutionResult.metadata` 写出逻辑（`schema: "executionTaskSnapshot"`）；验证：为合法/非法/超限 snapshot 增加工具单测并运行 `cargo test -p astrcode-adapter-tools --lib task_write`
- [x] 1.3 在 `crates/server/src/bootstrap/capabilities.rs` 注册 `taskWrite`，确认 `SideEffect::Local` 使其在 code mode 可见、plan/review mode 不可见；验证：补充 capability surface / mode 相关测试并运行对应 Rust 单测

## 2. Task 投影、恢复与 prompt 注入

- [x] 2.1 在 `crates/session-runtime/src/state/mod.rs` 的 `SessionState` 新增 `active_tasks: StdMutex<HashMap<String, TaskSnapshot>>` 字段，在 `translate_store_and_cache()` 中拦截 `tool_name == "taskWrite"` 的 `ToolResult` 事件，从 `metadata` 提取 snapshot 并更新对应 owner 的缓存条目；验证：补充 replay / clear / owner 隔离测试并运行 `cargo test -p astrcode-session-runtime --lib`
- [x] 2.2 在 `crates/session-runtime/src/query/service.rs` 的 `SessionQueries` 新增 `active_task_snapshot(session_id, owner)` 查询方法；验证：增加 query 方法单测
- [x] 2.3 在 `crates/session-runtime/src/turn/request.rs` 的 `build_prompt_output()` 中新增 `live_task_snapshot_declaration(...)`，参照 `live_direct_child_snapshot_declaration` 模式，只注入 `in_progress` + `pending` 项；验证：新增 turn/request 测试，确认 `taskWrite` 后下一步 prompt 已包含活跃 task 摘要、空列表时不生成声明

## 3. 应用层读模型与前端展示

- [x] 3.1 在 `crates/application` 的 `terminal_control_facts()` 中调用 `SessionQueries::active_task_snapshot()`，将结果映射到 `TerminalControlFacts.active_tasks` 字段；验证：补充应用层用例测试
- [x] 3.2 在 `crates/protocol/src/http/conversation/v1.rs` 的 `ConversationControlStateDto` 新增 `activeTasks: Option<Vec<TaskItemDto>>` 字段；在 `crates/server/src/http/terminal_projection.rs` 的 `to_conversation_control_state_dto()` 映射中填充该字段；验证：`cargo test -p astrcode-protocol`
- [x] 3.3 扩展 `frontend/src/types.ts` 的 `ConversationControlState` 新增 `activeTasks` 字段，更新 `frontend/src/lib/api/conversation.ts` 的 DTO 映射；验证：`cd frontend && npm run typecheck`
- [x] 3.4 新增前端 task 卡片组件（对话区顶部折叠卡片），展示当前 in_progress 任务标题 + pending/completed 计数，在 `activeTasks` 为 `None` 时自动隐藏，接入 conversation control state 的 hydration 和 `UpdateControlState` delta；验证：`cd frontend && npm run lint`

## 4. 回归验证与收尾

- [x] 4.1 为 `taskWrite` 与 `session_plan` 的边界增加回归测试，确认调用 `taskWrite` 不会修改 canonical plan 文件、plan 状态或 plan surface；确认 `taskWrite` 调用在 transcript 中作为正常 ToolCallBlock 出现（不被抑制）；验证：运行相关 Rust 单测
- [x] 4.2 为 conversation hydration / delta 的 task panel 行为增加端到端读模型测试，确认客户端不需要扫描 transcript 即可恢复 task 状态；验证：运行 `cargo test -p astrcode-session-runtime --lib` 和前端测试
- [x] 4.3 运行仓库级格式与边界检查，确保新增 task 系统未破坏分层约束；验证：`cargo fmt --all`、`cargo clippy --all-targets --all-features -- -D warnings`、`node scripts/check-crate-boundaries.mjs`
