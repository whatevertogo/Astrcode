## 1. 恢复语义与状态接线

- [x] 1.1 在 `crates/core` 中增加输出截断 continuation 所需的 synthetic prompt origin 或等价稳定语义
- [x] 1.2 在 `crates/session-runtime/src/turn/` 中新增 continuation 恢复模块，并复用 `max_output_continuation_attempts`
- [x] 1.3 调整 `crates/session-runtime/src/turn/runner/step.rs`，让 `LlmFinishReason::MaxTokens` 走正式恢复路径而不是只打 warning

## 2. 汇总与边界保护

- [x] 2.1 扩展 `crates/session-runtime/src/turn/summary.rs`，记录截断恢复次数与最终停止原因
- [x] 2.2 明确存在 tool calls、取消或达到上限时的退出策略，并保证可恢复中的中间截断不会被过早当作最终失败释放
- [x] 2.3 核对 token 预算、message projection、replay 路径对新 synthetic prompt 语义的兼容性

## 3. 验证

- [x] 3.1 为 `crates/session-runtime/src/turn/runner/step.rs` 增加 `max_tokens` 恢复测试
- [x] 3.2 运行 `cargo test -p astrcode-session-runtime`
- [x] 3.3 运行 `cargo clippy -p astrcode-session-runtime --all-targets --all-features -- -D warnings`
