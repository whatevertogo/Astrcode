## 1. Token Budget 闭环

- [x] 1.1 在 `crates/session-runtime/src/turn/runner.rs` 接入 `check_token_budget`，让 turn 在 step 循环中根据预算决定继续或结束
- [x] 1.2 在 `crates/session-runtime/src/turn/runner.rs` 注入 auto-continue nudge，并记录 continuation 次数与停止原因
- [x] 1.3 为 budget 驱动的 auto-continue 增加测试，覆盖”继续””达到上限””收益递减停止”

## 2. Turn Observability

- [x] 2.1 在 `crates/session-runtime` 中定义 turn 级稳定汇总结构，承接耗时、cache reuse、continuation、compaction 命中
- [x] 2.2 在 `crates/session-runtime/src/turn/runner.rs` 和相关上下文模块汇入原始事件数据，生成汇总结果
- [x] 2.3 将 turn 汇总接到治理/诊断读取路径，避免上层重新扫描整条事件流

## 3. Compaction Tail 定稿

- [x] 3.1 审视 `recent_turn_event_tail` 与现有 recent stored events 是否足以表达 compaction tail
- [x] 3.2 若现有表达不足，再在 `crates/session-runtime` 中补最小显式快照结构；若足够，则仅固化契约与测试
- [x] 3.3 补充测试覆盖自动压缩前保留 tail、压缩后恢复关键上下文的行为

## 4. 验证

- [x] 4.1 运行 `cargo check -p astrcode-session-runtime`
- [x] 4.2 运行 `cargo test -p astrcode-session-runtime`
- [x] 4.3 运行 `cargo clippy -p astrcode-session-runtime --all-targets --all-features -- -D warnings`
