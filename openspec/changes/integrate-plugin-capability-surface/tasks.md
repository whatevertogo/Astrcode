## 1. 把 plugin 接入组合根与治理主链

- [ ] 1.1 盘点当前项目与旧项目中的 plugin discovery、loader、supervisor、hook、skill、capability 入口。
- [ ] 1.2 在 `server/bootstrap` 中建立插件发现、装载、物化的独立装配模块，避免把细节堆进主组合根。
- [ ] 1.3 将 plugin 生命周期接入 `application` 的 reload 与治理视图。

## 2. 统一并入 capability surface

- [ ] 2.1 建立 plugin capability / skill / hook 的物化流程，使其能转换为统一能力面输入。
- [ ] 2.2 让 `kernel` 的 surface 替换链路同时接收 builtin、MCP、plugin 三类来源。
- [ ] 2.3 确保 reload 后真正发生整份 surface 替换，而不是 manager 内部半刷新。

## 3. 验证插件迁移不破坏架构

- [ ] 3.1 为 plugin lifecycle、surface 替换和治理快照编写测试。
- [ ] 3.2 验证 plugin 失败信息不会被静默吞掉。
- [ ] 3.3 运行 `cargo fmt --all --check`。
- [ ] 3.4 运行 `cargo clippy --all-targets --all-features -- -D warnings`。
- [ ] 3.5 运行 `cargo test`。
