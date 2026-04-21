## 1. 收口 `core` 的职责边界

- [x] 1.1 调整 `crates/core/src/error.rs` 与相关调用点，把 `AstrError::HttpRequest` 从 `reqwest::Error` 解耦为中立错误载体，并移除 `astrcode-core` 对 `reqwest` 的依赖；验证：`cargo check -p astrcode-core`
- [x] 1.2 将 `crates/core/src/runtime/coordinator.rs` 的 owner 迁到 `crates/server/src/bootstrap` 附近，保留 `crates/core/src/runtime/traits.rs` 中的纯契约，并清理 `crates/core/src/runtime/mod.rs` / `crates/core/src/lib.rs` 的导出；验证：`cargo check -p astrcode-core -p astrcode-server`
- [x] 1.3 拆分 `crates/core/src/agent/mod.rs` 为按职责组织的子模块，保持对外语义与导出路径稳定，同时删除迁移后遗留的死代码；验证：`cargo test -p astrcode-core --lib`

## 2. 让 `session-runtime` 完整拥有会话 projection 真相

- [x] 2.1 从 `crates/core/src/agent/input_queue.rs` 移出 `replay_index`、`replay_for_agent`、`apply_event_for_agent`，在 `crates/session-runtime/src/state/input_queue.rs`、`crates/session-runtime/src/state/projection_registry.rs`、`crates/session-runtime/src/query/input_queue.rs` 中接管这些算法，同时保留 `InputQueueProjection` DTO 定义在 core；验证：`cargo test -p astrcode-session-runtime input_queue --lib` 与 `cargo check -p astrcode-core`
- [x] 2.2 保持 `TurnProjectionSnapshot` 作为共享 checkpoint 载体暂留 core，同时确保 `crates/session-runtime/src/turn/projector.rs`、`crates/session-runtime/src/query/turn.rs`、`crates/session-runtime/src/turn/watcher.rs` 继续作为唯一业务 owner，并清理本次迁移中的错误 owner 假设；验证：`cargo test -p astrcode-session-runtime turn --lib`

## 3. 把环境副作用下沉到 adapter

- [x] 3.1 为 tool result persist、project/home 路径解析、shell 检测、plugin manifest 解析补齐或收紧稳定契约，修改范围覆盖 `crates/core/src/lib.rs`、`crates/support/src/lib.rs` 与相应调用接口；验证：`cargo check -p astrcode-core -p astrcode-support -p astrcode-application -p astrcode-session-runtime`
- [x] 3.2 将 `crates/core/src/tool_result_persist.rs` 拆成“共享协议 + 共享宿主实现”两层：core 保留 DTO、常量与纯解析 helper，把 `persist_tool_result`、`maybe_persist_tool_result`、磁盘写入逻辑迁入 `crates/support/src/tool_results.rs`，并更新 `crates/session-runtime/src/turn/tool_result_budget.rs`、`crates/adapter-tools/src/builtin_tools/*`、`crates/adapter-mcp/src/bridge/resource_tool.rs` 等调用方；验证：`cargo test -p astrcode-session-runtime --lib`、`cargo check -p astrcode-support -p astrcode-adapter-tools -p astrcode-adapter-mcp`
- [x] 3.3 将 `crates/core/src/shell.rs` 拆成“共享 shell 类型 + 共享宿主实现”两层，把检测函数迁入 `crates/support/src/shell.rs`，并更新 `crates/adapter-prompt/src/context.rs`、`crates/adapter-tools/src/builtin_tools/shell.rs` 等调用方；验证：`cargo check -p astrcode-core -p astrcode-support -p astrcode-adapter-tools -p astrcode-adapter-prompt`
- [x] 3.4 新增 `crates/support/src/hostpaths/`，将 `crates/core/src/project.rs`、`crates/core/src/home.rs` 中的 `canonicalize` / home 目录解析 owner 迁出 core，保留纯 project identity 算法在 core，并更新 `crates/adapter-storage/src/session/paths.rs`、`crates/session-runtime/src/state/paths.rs`、`crates/server/src/bootstrap/*`、`crates/cli/src/launcher` 等调用方；验证：`cargo check -p astrcode-support -p astrcode-core -p astrcode-server -p astrcode-session-runtime -p astrcode-adapter-storage -p astrcode-cli`
- [x] 3.5 将 `crates/core/src/plugin/manifest.rs` 的 TOML 解析迁出 core，保留 `PluginManifest` 数据结构，并移除 `astrcode-core` 对 `toml` 的依赖；验证：`cargo check -p astrcode-core` 与相关 manifest 加载测试

## 4. 迁移应用层治理与调用路径

- [x] 4.1 更新 `crates/application/src/lifecycle/governance.rs`、`crates/application/src/lifecycle/mod.rs` 与 `crates/server/src/bootstrap/governance.rs`，让 `application` 只通过治理端口消费运行时协调，而由 `server` 组合根拥有 `RuntimeCoordinator` 设施 owner；验证：`cargo check -p astrcode-application -p astrcode-server`
- [x] 4.2 把 `crates/application/src/session_use_cases.rs`、`crates/application/src/execution/profiles.rs` 等路径相关用例改为通过 `astrcode-support::hostpaths` 等稳定契约编排 project dir / working dir / home 能力，不再直接依赖 core-owned helper；验证：`cargo test -p astrcode-application --lib`
- [x] 4.3 回归治理、路径与会话相关 server/application 测试，确认 `server` 仍只依赖稳定应用层接口，`application` 不重新持有组合根设施；验证：`cargo test -p astrcode-server` 与 `cargo test -p astrcode-application`

## 5. 文档与架构守卫

- [x] 5.1 更新 `PROJECT_ARCHITECTURE.md` 与必要的 crate 级文档，明确 `core`、`session-runtime`、`application`、`server`、`adapter-*`、`astrcode-support` 的新 owner 边界与数据流，并记录 `TurnProjectionSnapshot` / `tokio sender` 的延期原因；验证：人工审阅文档与本 change artifacts 一致
- [x] 5.2 运行架构与编译校验，确认迁移后依赖方向与边界约束成立，且 `astrcode-core` 已移除 `reqwest`、`dirs`、`toml` 依赖；验证：`cargo check --workspace`、`cargo test --workspace --exclude astrcode --lib`、`node scripts/check-crate-boundaries.mjs`、人工检查 `crates/core/Cargo.toml`
