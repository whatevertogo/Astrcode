## 1. 采集器与接线

- [x] 1.1 在 `crates/application/src/observability/*` 及必要的底层契约位置恢复真实 observability 采集器与窄 recorder 接口，替换当前仅有 snapshot 类型的占位状态。
- [x] 1.2 在 `crates/server/src/bootstrap/governance.rs`、`crates/server/src/bootstrap/runtime.rs` 把真实采集器接入治理快照，移除默认零值观测器作为常态实现。

## 2. 指标记录路径

- [x] 2.1 在会话加载与流恢复路径补齐 session rehydrate、SSE catch-up 指标记录，涉及 `crates/session-runtime/src/query/*`、`crates/server/src/http/routes/sessions/*` 或对应恢复路径。
- [x] 2.2 在 `crates/session-runtime/src/turn/*`、`crates/application/src/agent/*` 等执行路径补齐 turn、subrun、delivery diagnostics 的记录，并确保失败路径同样计数。

## 3. 测试与验收

- [x] 3.1 为 observability 管线补充单元与集成测试，验证治理快照会随真实行为变化，而不是固定返回零值。
- [x] 3.2 校验 `crates/server/src/http/mapper.rs` 与相关状态接口输出，确认 DTO 不承载业务逻辑；验证命令：`cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
