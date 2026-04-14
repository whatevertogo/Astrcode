## 1. 统一 child turn terminal finalizer

- [x] 1.1 在 `crates/application/src/agent/terminal.rs` 提炼统一的 child turn terminal finalizer context 与 helper。
- [x] 1.2 将 spawn 与 idle-resume 路径切到统一 watcher / finalizer。
- [x] 1.3 调整 terminal notification 构造与 parent routing，显式使用 `parent_session_id / parent_turn_id`。
- [x] 1.4 调整 terminal notification id 规则，避免同一 agent 多轮 turn 被错误去重。

## 2. 打通 wake 路径的逐级冒泡

- [x] 2.1 修改 `crates/application/src/agent/wake.rs`，让 wake completion 在 batch `acked / consume` 之前尝试把当前 agent 这一轮继续向上冒泡。
- [x] 2.2 保持 batch `requeue` 语义：上行 finalizer 失败时，不得把当前批次伪装成已成功消费。
- [x] 2.3 明确 root agent 不参与 child 上行 finalization，只消费当前 batch。

## 3. 补齐回归测试

- [x] 3.1 增加“显式 parent session 路由，不再依赖 `child_ref.session_id`”测试。
- [x] 3.2 增加“middle 不等待 descendant settled，先汇报本轮结果”测试。
- [x] 3.3 增加 `leaf -> middle -> root` wake bubbling 回归测试。
- [x] 3.4 保持既有 busy-parent、wake-failure、durable-recovery 测试通过。

## 4. 文档与验证

- [x] 4.1 为本次修复补充 OpenSpec proposal / design / spec delta。
- [x] 4.2 运行 `cargo fmt --all`。
- [x] 4.3 运行 `cargo test -p astrcode-application agent:: --lib`。
