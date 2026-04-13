## 1. 执行子域结构

- [x] 1.1 在 `crates/application/src/execution/` 建立执行子域，并让 `App` 通过薄 façade 委托实现
- [x] 1.2 将执行相关共享类型与错误映射整理到 `application` 内稳定位置，避免继续堆进 `App` 根文件

## 2. 根代理执行

- [x] 2.1 实现根代理执行入口，完成参数校验、working-dir 规范化、profile 解析与 session 准备
- [x] 2.2 将根代理执行接回 `crates/server/src/http/routes/agents.rs`
- [x] 2.3 为根代理执行补测试，覆盖成功、profile 不存在、非法输入

## 3. 子代理执行

- [x] 3.1 实现子代理执行入口，完成 spawn 参数解析、control 协调和 turn 启动
- [x] 3.2 将 send/observe/close 的业务编排接回稳定入口，避免路由直接拼底层对象
- [x] 3.3 为子代理结果回流与关闭路径补测试

## 4. Profile 解析与缓存

- [x] 4.1 增加 working-dir 级 profile 解析与缓存
- [x] 4.2 增加缓存命中、agent 缺失、缓存失效相关测试

## 5. 验证

- [x] 5.1 运行 `cargo check -p astrcode-application`
- [x] 5.2 运行 `cargo test -p astrcode-application -p astrcode-server`
- [x] 5.3 运行 `cargo clippy -p astrcode-application -p astrcode-server --all-targets --all-features -- -D warnings`
