## 1. 执行控制合同

- [ ] 1.1 在 `crates/protocol/src/http/agent.rs`、`crates/protocol/src/http/event.rs`、`frontend/src/types.ts`、`frontend/src/lib/api/*` 引入统一的执行控制 DTO，正式承载 `tokenBudget`、`maxSteps` 与手动 compact 控制。
- [ ] 1.2 在 `crates/application/src/execution/*`、`crates/application/src/agent/*` 增加执行控制参数校验与错误归类，确保根代理、子代理和普通会话入口共享一致语义。

## 2. 服务端执行语义

- [ ] 2.1 在 `crates/session-runtime/src/turn/*` 与相关状态/事件路径中接入显式 `tokenBudget`，让本次执行可以覆盖默认 budget 且不污染全局配置。
- [ ] 2.2 为运行中 session 的手动 compact 建立正式处理路径，优先在 `crates/application/src/agent/*`、`crates/session-runtime/src/state/*`、`crates/session-runtime/src/turn/*` 收口为服务端真相而不是前端本地排队。

## 3. 前端交互与验证

- [ ] 3.1 更新 `frontend/src/components/Chat/InputBar.tsx`、`frontend/src/hooks/app/useComposerActions.ts`、`frontend/src/hooks/useAgent.ts` 的 composer 控制流程，展示执行控制被接受、延迟执行或拒绝的稳定反馈。
- [ ] 3.2 为前后端合同补齐测试，至少覆盖显式 budget 透传、非法控制参数拒绝、busy 状态下 manual compact 处理；验证命令：`npm run check:frontend:push && cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
