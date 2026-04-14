## 1. 治理入口收口

- [x] 1.1 在 `crates/application/src/lifecycle/governance.rs`、`crates/server/src/bootstrap/governance.rs`、`crates/server/src/bootstrap/runtime.rs` 为 `AppGovernance` 接入真实 `RuntimeReloader`，让组合根不再返回未接线的治理模型。
- [x] 1.2 调整 `crates/server/src/http/routes/config.rs`、`crates/server/src/http/mapper.rs` 与必要的 protocol DTO，使 `POST /api/config/reload` 走统一治理 reload 语义而不是仅重读配置。

## 2. 统一 reload 实现

- [x] 2.1 在 `crates/server/src/bootstrap/*` 与 `crates/adapter-mcp/src/manager/*` 收口 builtin / MCP / plugin 的候选 surface 组装与一次性替换流程，确保失败时保留旧 surface。
- [x] 2.2 在治理 reload 中加入运行中 session 冲突检查，并将拒绝原因通过 `application` 错误和 server 响应稳定暴露。

## 3. 验证与文档

- [x] 3.1 为治理 reload 增加回归测试，覆盖完整刷新成功、运行中 session 拒绝、候选 surface 失败后保留旧状态等场景。
- [x] 3.2 更新 `PROJECT_ARCHITECTURE.md`、`README.md` 与相关注释，明确新的治理入口、reload 语义和回滚策略；验证命令：`cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
