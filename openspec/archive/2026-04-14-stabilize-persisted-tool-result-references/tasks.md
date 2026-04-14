## 1. Durable replacement 事件与恢复状态

- [x] 1.1 在 `crates/core/src/event/types.rs` 与相关翻译/投影模块中新增 `ToolResultReferenceApplied` 事件，记录 `tool_call_id`、`replacement`、`persisted_relative_path` 与 `original_bytes`
- [x] 1.2 在 `crates/session-runtime` 的 turn 执行上下文中引入 `ToolResultReplacementState`，支持 `must_reapply` / `frozen` / `fresh` 三类决策
- [x] 1.3 补充 session 恢复与 replay 测试，确保仅靠 durable 事件即可重建 replacement state

## 2. Aggregate budget 接入 request assembly

- [x] 2.1 在 `crates/session-runtime/src/turn/request.rs` 中增加 aggregate tool-result budget 阶段，并固定在 `micro_compact` / `prune_pass` 之前执行
- [x] 2.2 复用 `crates/core/src/tool_result_persist.rs` 的 `<persisted-output>` 契约，对同一 API-level user tool-result 批次执行 largest-first 的 fresh replacement
- [x] 2.3 显式跳过已是 `<persisted-output>`、非文本结果和不应参与 aggregate replacement 的结果，避免重复 compact

## 3. 回读契约与工具联动

- [x] 3.1 确认 `crates/adapter-tools/src/builtin_tools/read_file.rs` 与 `fs_common.rs` 的 `tool-results/**` 读取契约继续成立，并为 persisted path 回读补回归测试
- [x] 3.2 审视 `grep`、`shell`、`findFiles` 等已使用 persisted-output 的工具，确保它们与 aggregate replacement 组合后仍输出一致的引用格式
- [x] 3.3 明确 request 层不新增新的 feedback package / summary 协议，避免与 persisted-output 主路径混用

## 4. Observability 与验证

- [x] 4.1 扩展 turn 汇总与 observability，记录 replacement 命中数、重放数、节省字节数与 over-budget message 数
- [x] 4.2 为 `crates/session-runtime` 增加 aggregate budget、resume 重放、persisted reread 相关测试
- [x] 4.3 运行 `cargo test -p astrcode-session-runtime`
- [x] 4.4 运行 `cargo clippy -p astrcode-session-runtime --all-targets --all-features -- -D warnings`
