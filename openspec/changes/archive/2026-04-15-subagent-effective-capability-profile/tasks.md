## 1. Spawn Grant 与 Capability Surface

- [x] 1.1 在 `crates/core/src/agent/mod.rs` 为 `SpawnAgentParams` 增加 task-scoped capability grant，并补充参数校验与 DTO 映射。
- [x] 1.2 在 `crates/application/src/execution` / `crates/application/src/agent` 增加 child resolved capability surface 求交逻辑，输入至少包括 parent 可继承工具面、spawn grant 与 runtime availability。

## 2. Prompt 与 Runtime 对齐

- [x] 2.1 修改 `crates/kernel/src/registry/router.rs` 与相关调用链，提供 child 可复用的 filtered capability router 入口。
- [x] 2.2 修改 `crates/server/src/bootstrap/prompt_facts.rs` 与相关 prompt build 输入，使 child prompt 只暴露 resolved capability surface 对应的 capability / tool guides。
- [x] 2.3 修改 `crates/session-runtime/src/turn/runner.rs`、`crates/session-runtime/src/turn/tool_cycle.rs` 等执行链路，使 runtime 使用与 prompt 相同的 filtered capability router。
- [x] 2.4 为“prompt 可见工具集合 == runtime 可执行工具集合”补充单元测试或集成测试。

## 3. 生命周期与状态可见性

- [x] 3.1 在 child launch / subrun lifecycle 中接线 `ResolvedExecutionLimitsSnapshot`，把 launch-time resolved capability snapshot 写入 durable 事件。
- [x] 3.2 修改 `crates/server/src/http/routes/agents.rs`、`crates/server/src/http/mapper.rs` 和相关 read-model，使 subrun status 返回 `resolved_limits`。
- [x] 3.3 为“status 从 durable 历史恢复 resolved limits”补充契约测试。

## 4. Prompt Governance 与设计文档

- [x] 4.1 更新 `crates/adapter-tools/src/agent_tools/spawn_tool.rs` 及必要的协作 guidance，使 `spawn` 明确区分 profile 行为模板与 capability grant 任务授权。
- [x] 4.2 更新相关架构/设计文档，说明 `profile / spawn grant / resolved capability surface / policy engine` 的职责分配。

## 5. 验证

- [x] 5.1 运行 `cargo fmt --all`。
- [x] 5.2 运行 `cargo clippy --all-targets --all-features -- -D warnings`。
- [x] 5.3 运行 `cargo test --workspace`，至少覆盖 spawn grant、subagent execution、subrun status、prompt facts 相关测试。
