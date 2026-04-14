## 1. Loop 语义建模

- [x] 1.1 在 `crates/session-runtime/src/turn/` 中引入显式的 transition / stop cause 类型，并接入 `TurnExecutionContext`
- [x] 1.2 调整 `crates/session-runtime/src/turn/runner.rs` 与 `crates/session-runtime/src/turn/runner/step.rs`，让 continue/end 路径更新显式原因而不是只依赖隐式分支
- [x] 1.3 把 budget-driven auto-continue 接到新的 transition 模型，补齐现有 spec 与实现之间的缺口

## 2. 汇总与事件接线

- [x] 2.1 扩展 `crates/session-runtime/src/turn/summary.rs`，让 `TurnSummary` 暴露结构化 transition / stop cause
- [x] 2.2 审视 `crates/session-runtime/src/turn/events.rs` 中 `TurnDone` / 相关事件的 reason 语义，必要时补稳定映射
- [x] 2.3 确认 `application` 和 `server` 无需理解新的内部 loop 细节，不引入新的跨层依赖

## 3. 验证

- [x] 3.1 为 `crates/session-runtime/src/turn/runner/step.rs` 增加 transition-oriented 单元测试
- [x] 3.2 运行 `cargo test -p astrcode-session-runtime`
- [x] 3.3 运行 `cargo clippy -p astrcode-session-runtime --all-targets --all-features -- -D warnings`
