## 1. Root 执行入口收口

- [x] 1.1 调整 [`crates/application/src/lib.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/lib.rs) 与 [`crates/application/src/execution/root.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/execution/root.rs)，提供正式的 root execution 用例入口并接收显式执行控制
- [x] 1.2 修改 [`crates/server/src/http/routes/agents.rs`](/d:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/agents.rs)，让 `POST /api/v1/agents/{id}/execute` 委托 `application` 正式入口，而不是自行 `create_session + submit_prompt`

## 2. Subagent profile 驱动执行

- [x] 2.1 在 [`crates/application/src/execution/profiles.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/execution/profiles.rs) 定义并接线执行侧 profile resolver 的正式使用路径
- [x] 2.2 修改 [`crates/application/src/agent/mod.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/agent/mod.rs) 与 [`crates/application/src/execution/subagent.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/execution/subagent.rs)，移除占位 profile 构造，改为先解析并校验真实 profile 再创建 child session
- [x] 2.3 恢复 root/subagent 执行对 profile 存在性、mode、工具约束与模型偏好的正式消费，并确保无效 profile 不产生 session / agent 注册副作用

## 3. 验证与回归

- [x] 3.1 为 root 执行补充路由与用例测试，覆盖 profile 不存在、mode 不允许、显式 execution control 透传
- [x] 3.2 为 subagent 执行补充测试，覆盖真实 profile 解析、无效 profile 不创建 child session、工具与模型约束进入执行输入
- [x] 3.3 运行并记录验证命令：`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`
