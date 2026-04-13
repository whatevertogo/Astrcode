## 1. Child 终态到 delivery 管线接线

- [x] 1.1 在 [`crates/application/src/execution/subagent.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/execution/subagent.rs) 或相邻 child finalize 边界补齐 terminal notification 生成与调用点
- [x] 1.2 将 [`crates/application/src/agent/wake.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/agent/wake.rs) 接到 child completion 主链，确保 Delivered / Failed / Closed 都能进入 parent delivery queue

## 2. Parent wake 与缓冲语义收口

- [x] 2.1 对齐 [`crates/kernel/src/agent_tree/mod.rs`](/d:/GitObjectsOwn/Astrcode/crates/kernel/src/agent_tree/mod.rs) 中 parent delivery batch 的 checkout / consume / requeue 语义，保证 busy / failed 路径不会提前消费
- [x] 2.2 调整 [`crates/application/src/agent/wake.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/agent/wake.rs) 的父级 wake 流程，明确繁忙父级延迟、提交失败重排与继续 drain 的行为
- [x] 2.3 确保 `ChildSessionNotification` 的摘要、最终回复摘录与 wake prompt 使用一致的数据来源，避免 UI 投影与父级输入漂移

## 3. 可观测性与验证

- [x] 3.1 将 parent wake 成功、失败、重排与批次消费行为接入现有 observability 记录点，避免静默失败
- [x] 3.2 补充回归测试，覆盖 child 完成触发父级 wake、父级繁忙重试、wake 失败 requeue、重复消费保护
- [x] 3.3 运行并记录验证命令：`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`
