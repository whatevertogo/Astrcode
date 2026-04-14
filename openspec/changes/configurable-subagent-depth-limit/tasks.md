## 1. Implementation

- [x] 1.1 统一 runtime 默认子代理深度为 3
- [x] 1.2 将 agent depth limit 从配置显式注入 kernel 组合根
- [x] 1.3 将当前 depth limit 暴露到 prompt vars / metadata
- [x] 1.4 更新协作 guidance 与 spawn tool 文案，强调复用 idle child
- [x] 1.5 改进 depth / concurrency 超限时的 spawn 错误映射

## 2. Validation

- [x] 2.1 `cargo fmt --all`
- [x] 2.2 `cargo test --workspace`
- [x] 2.3 `cargo clippy --all-targets --all-features -- -D warnings`
