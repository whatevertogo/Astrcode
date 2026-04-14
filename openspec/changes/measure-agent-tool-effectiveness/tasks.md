## 1. Collaboration Fact Model

- [x] 1.1 定义 agent collaboration 原始事实模型，并在 `crates/application/src/agent/*` 与必要的 session 存储边界接入 `spawn / send / observe / close / delivery` 记录点。
- [x] 1.2 为协作事实补充策略上下文字段或稳定引用，覆盖 prompt/policy revision、`max_subrun_depth` 与 `max_spawn_per_turn`。
- [x] 1.3 为失败与拒绝路径补充测试，确保缺失所有权、命中限制和重放场景都能产生可诊断事实。

## 2. Derived Read Models

- [x] 2.1 在 `crates/session-runtime/src/turn/*` 扩展 turn summary，纳入 collaboration summary 与关键效率字段。
- [x] 2.2 在 `crates/application/src/observability/*` 扩展 runtime observability collector/read model，聚合 child reuse、observe-to-action、spawn-to-delivery、orphan child 与 delivery latency 等指标。
- [x] 2.3 在 `crates/server/src/bootstrap/*` 与相关 HTTP/治理读取面暴露纯数据 DTO，供调试或治理视图消费。

## 3. Validation

- [x] 3.1 补充单元与集成测试，验证原始协作事实、turn summary 和全局 observability snapshot 之间的一致性。
- [x] 3.2 准备最小调试脚本或手动验收步骤，覆盖“过度 spawn”“observe 轮询”“child reuse”三类典型场景。
- [x] 3.3 运行 `cargo fmt --all`、`cargo clippy --all-targets --all-features -- -D warnings` 与 `cargo test --workspace`，确认评估管线不破坏现有执行路径。
