## 1. Watch 组合根接线

- [x] 1.1 在 server 组合根中装配 `WatchService` 与对应 `WatchPort`，并让 agent definition source 进入真实运行路径
- [x] 1.2 基于当前工作目录集合维护 `WatchSource::AgentDefinitions`，明确全局与项目级 source 的注册和移除时机

## 2. Profile cache 失效链路

- [x] 2.1 将 watch 事件映射到 [`crates/application/src/execution/profiles.rs`](/d:/GitObjectsOwn/Astrcode/crates/application/src/execution/profiles.rs) 的 `invalidate`、`invalidate_global` 或 `invalidate_all`
- [x] 2.2 确保 profile 文件变化只影响后续解析与后续执行，不会改写正在运行中的 turn 或已启动 child session
- [x] 2.3 对齐执行侧读取路径，验证 cache 失效后后续 root/subagent 执行重新读取新的 profile 结果

## 3. 验证与平台行为

- [x] 3.1 补充测试，覆盖 scoped/global profile 变化触发失效、无法精确归属时的保守失效、运行中 turn 不受影响
- [x] 3.2 记录必要日志与手动验收步骤，验证 profile 更新后无需重启即可影响后续执行
- [x] 3.3 运行并记录验证命令：`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`
