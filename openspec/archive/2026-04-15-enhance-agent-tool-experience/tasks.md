## 1. Delegation Prompt Surface

- [x] 1.1 增强 `crates/adapter-prompt/src/contributors/workflow_examples.rs`，让共享协作协议正式区分 fresh child、resumed child、restricted child 三种 delegation mode
- [x] 1.2 增强 `crates/adapter-prompt/src/contributors/agent_profile_summary.rs`，把当前 child-eligible profile 列表收口成行为模板目录，而不是 capability 目录

## 2. Collaboration Guidance And Tool Descriptions

- [x] 2.1 收紧 `crates/adapter-tools/src/agent_tools/{spawn_tool,send_tool,observe_tool,close_tool}.rs` 的 description，只保留动作级 guidance，避免继续在单工具 description 中内联目录或 child contract
- [x] 2.2 在 `crates/adapter-prompt/src/contributors/prompt_declaration.rs` 相关链路和 child launch / resume 路径中注入 child-scoped execution contract，并补充 fresh / resume / restricted 三类测试

## 3. Execution And Result Projection

- [x] 3.1 在 `crates/application/src/execution/subagent.rs`、`crates/application/src/agent/routing.rs` 与相关调用链中补 delegation metadata，保证 child launch / reuse 能产出 responsibility continuity 与 restricted-child contract 所需信息
- [x] 3.2 在 `crates/adapter-tools/src/agent_tools/collab_result_mapping.rs` 及相关 observe / result 路径中增加 reuse / close / respawn advisory projection，并覆盖 idle-reusable、idle-mismatch、restricted-child 三类场景测试

## 4. Validation And Documentation

- [x] 4.1 更新需要同步的架构 / 设计文档，明确共享协作协议、行为模板目录、child execution contract 与 advisory projection 的职责边界
- [x] 4.2 运行 `cargo fmt --all`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace` 验证实现与协作路径测试
