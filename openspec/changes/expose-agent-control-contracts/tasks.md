## 1. 稳定化 subrun status 合同

- [ ] 1.1 盘点当前 `kernel`、`application`、`server` 中与 root agent / subrun 状态相关的入口与内部依赖点。
- [ ] 1.2 在 `kernel` 中引入稳定的状态查询类型与查询接口，禁止上层依赖 `agent_tree` 内部节点结构。
- [ ] 1.3 将 `application` 对 agent status 的编排切到稳定控制合同，并统一错误映射。
- [ ] 1.4 将 `server` 的对应路由切到 `application` 暴露的稳定用例接口。

## 2. 收敛 route / wake / close / observe 合同

- [ ] 2.1 识别旧项目中确实属于真实产品合同的 route / wake / close / observe 行为。
- [ ] 2.2 为保留的行为定义稳定输入输出，不暴露内部 mailbox、事件总线或内部节点结构。
- [ ] 2.3 在 `kernel` 中落地稳定控制接口，并让 `application` 负责参数校验与错误归类。
- [ ] 2.4 删除或拒绝继续传播依赖内部实现的临时 façade。

## 3. 验证边界不回潮

- [ ] 3.1 为 `kernel` 编写控制合同测试，覆盖 subrun status、delivery、wake、close、observe。
- [ ] 3.2 为 `application/server` 编写合同测试，验证上层不再直接依赖 `agent_tree` 内部结构。
- [ ] 3.3 运行 `cargo fmt --all --check`。
- [ ] 3.4 运行 `cargo clippy --all-targets --all-features -- -D warnings`。
- [ ] 3.5 运行 `cargo test`。
