## 1. Prompt Governance

- [x] 1.1 在 `crates/adapter-prompt/src/contributors/workflow_examples.rs` 收口系统级 collaboration guide，统一四工具状态机与默认行为。
- [x] 1.2 在 `crates/adapter-tools/src/agent_tools/{spawn_tool,send_tool,observe_tool,close_tool}.rs` 缩减重复说明，保留动作导向的最小 prompt metadata。
- [x] 1.3 为 `crates/adapter-tools/src/agent_tools/tests.rs` 补充回归测试，验证四工具 prompt 约束不会回退到高噪声或错误引导。

## 2. Runtime Collaboration Contract

- [x] 2.1 在 `crates/application/src/agent/{mod,routing,observe}.rs` 明确 `send / observe / close` 的 direct-child 所有权与错误路径语义。
- [x] 2.2 在 `crates/application/src/agent/observe.rs` 为 observe 结果补齐决策友好的投影字段，并保持原始 lifecycle/outcome 事实不丢失。
- [x] 2.3 在 `crates/application/src/execution/subagent.rs` 与相关结果映射中统一 child 复用、排队和 cascade close 的稳定结果语义。

## 3. Presentation and Validation

- [x] 3.1 如需要，在 `frontend/src/hooks/app/*` 与 `frontend/src/store/*` 中将 child lineage 的默认展示策略与新的协作心智对齐。
- [x] 3.2 补充 application / adapter / frontend 级测试，覆盖 direct-child 拒绝、observe 决策投影、send 排队与 close 级联行为。
- [x] 3.3 运行 `cargo fmt --all`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace`，并在前端改动存在时运行 `cd frontend && npm run typecheck`。
