## 1. 核心类型与事件合同

- [ ] 1.1 在 `crates/core/src/action.rs`、`crates/core/src/event/domain.rs`、`crates/core/src/event/types.rs`、`crates/protocol/src/http/conversation/v1.rs` 引入后台任务通知、terminal session 纯数据结构与事件定义；验证：`cargo check --workspace`
- [ ] 1.2 扩展 `crates/session-runtime/src/query/conversation.rs` 与前端类型 `frontend/src/types.ts`，支持 background task notification block 和 terminal session block/patch；验证：`cargo test -p astrcode-session-runtime query::conversation` 与 `cd frontend && npm run typecheck`
- [ ] 1.3 更新 `PROJECT_ARCHITECTURE.md`，补充后台进程监管与 terminal session 的职责边界；验证：人工检查文档与本变更 design 一致

## 2. Claude 风格后台 shell

- [ ] 2.1 在 `crates/session-runtime` 中接入后台任务 started/completed/failed durable 事件与内部完成通知输入，不引入 suspended turn；验证：新增 `session-runtime` 单测覆盖后台任务完成通知 -> 新 turn 唤醒主路径
- [ ] 2.2 在 `crates/application/src/lifecycle/` 或相邻新模块实现 `ProcessSupervisor`/`AsyncTaskRegistry`，提供后台命令注册、完成通知、取消与 lost 终态上报；验证：`cargo test -p astrcode-application`
- [ ] 2.3 改造 `crates/adapter-tools/src/builtin_tools/shell.rs`，支持 `executionMode=auto|foreground|background`，返回 `backgroundTaskId` 与输出路径，并接入后台 shell 主路径；验证：新增 shell 工具集成测试，覆盖 foreground、background、cancel 三条路径

## 3. 持久终端会话工具族

- [ ] 3.1 新增持久执行工具模块，例如 `crates/adapter-tools/src/builtin_tools/exec_command.rs`、`write_stdin.rs`、`resize_terminal.rs`、`terminate_terminal.rs`、`close_stdin.rs`，定义参数与返回合同；验证：`cargo test -p astrcode-adapter-tools exec_command`
- [ ] 3.2 在 `crates/application` 与对应 adapter 层实现 PTY/pipe 驱动的 `TerminalSessionRegistry`，采用 `process_id` 持有活跃会话，支持 stdin 写入、stdout/stderr 流、退出码、关闭与 lost 语义；验证：新增跨平台可运行的单元/集成测试，至少覆盖启动、输入、退出、关闭
- [ ] 3.3 在 `crates/session-runtime` 接入 terminal session durable 事件、hydration 投影与 `process_id` 关联语义，输出主路径走 begin/delta/end 事件与 terminal interaction 记录；验证：新增 query/replay 测试覆盖 terminal session block、交互记录和长期运行会话主路径

## 4. 前端展示与验收

- [ ] 4.1 更新 `frontend/src/lib/toolDisplay.ts`、`frontend/src/components/Chat/ToolCallBlock.tsx` 及相关组件，为 background task notification 与 terminal session 提供稳定展示；验证：`cd frontend && npm run typecheck && npm run lint`
- [ ] 4.2 为 conversation surface 增加 background task / terminal session 的 API 与 SSE 验收样例，并补齐 `frontend` / `session-runtime` 快照测试；验证：`cargo test -p astrcode-session-runtime` 与 `cd frontend && npm run test -- --runInBand`
- [ ] 4.3 执行实现前总体验证清单：`cargo fmt --all`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace --exclude astrcode`、`node scripts/check-crate-boundaries.mjs`；验证：上述命令全部通过
