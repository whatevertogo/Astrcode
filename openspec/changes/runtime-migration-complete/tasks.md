## 1. 架构基础收口

- [x] 1.1 将单 session durable truth 从 `application` 收回 `session-runtime`，由 `SessionRuntime` 统一承接 create/list/delete、history/view/replay、submit/interrupt/compact
- [x] 1.2 为 `session-runtime` 增加 `query/`、`factory/` 等内部子域，避免把单 session 执行细节平铺在 crate 根
- [x] 1.3 扩展 `core::EventStore` 端口与 `adapter-storage` 实现，使其能表达 `ensure_session`、`list_session_metas`、`delete_sessions_by_working_dir`
- [x] 1.4 收瘦 `application::App`，移除 session shadow state，仅保留参数校验、配置读取、用例编排与错误映射
- [x] 1.5 运行 `cargo check`、`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test` 验证基础收口完成

## 2. Turn 编排完善（session-runtime）

- [x] 2.1 在 `crates/session-runtime/src/turn/token_budget.rs` 落地 token budget 解析与决策工具，保留纯单 session 语义
- [x] 2.2 在 `crates/session-runtime/src/turn/runner.rs` 保持多 step LLM → 工具 → LLM 循环，并通过 `request_assembler` 串接微压缩、裁剪与自动压缩
- [ ] 2.3 将 token budget 决策真正接入 `run_turn` 的 auto-continue 路径，补齐 continue nudge 注入与续写上限控制
- [ ] 2.4 补齐 turn 级 observability 汇总，把 prompt cache reuse、turn 耗时等指标从事件提升到稳定快照/治理视图
- [ ] 2.5 评估并补充 compaction tail 的显式快照结构；若现有 `recent_turn_event_tail` 足够，则更新 spec/design 而不是额外造层

## 3. Agent 控制面（kernel）

- [x] 3.1 在 `crates/kernel/src/agent_tree/mod.rs` 落地 inbox、observe、cancel propagation、等待唤醒等核心控制能力
- [x] 3.2 保持 agent 控制面留在 `kernel`，不把 mailbox / routing / cancel 真相下沉到 `session-runtime`
- [ ] 3.3 从当前 `agent_tree` 暴露稳定的 subrun status 查询入口，并接回 `application/server` 合同
- [ ] 3.4 若仍需要显式 `route` / `wake` façade，补成面向行为的公开方法；不要为了对应旧文件名拆出空模块

## 4. 执行业务入口（application）

- [ ] 4.1 新增 `application/execution` 子域，承接 `execute_root_agent` 与 `launch_subagent`，避免继续把执行用例堆进 `App` 根文件
- [ ] 4.2 实现根代理执行：参数解析 → profile 解析 → session 创建 → agent 注册/协调 → 异步 turn 执行
- [ ] 4.3 实现子代理执行：spawn/send/observe/close 的完整桥接，结果回流到父级 delivery 队列
- [ ] 4.4 在 `application` 边界提供按 working_dir 加载与缓存 agent profiles 的能力
- [ ] 4.5 审批、策略、权限判断继续留在 `application`；`session-runtime` 只消费已解析结果

## 5. Plugin 与动态能力面（server/bootstrap + kernel）

- [ ] 5.1 在 `server/bootstrap` 集成 plugin loader / supervisor，把插件生命周期纳入治理视图
- [ ] 5.2 将 plugin 贡献的 capabilities / skills 接到 kernel 的 capability surface 替换路径，而不是引入新的 runtime façade
- [ ] 5.3 若需要 hook 适配，保持适配发生在组合根，不让 `application` 或 `kernel` 直接依赖 plugin 细节
- [ ] 5.4 校验 plugin / MCP / builtin 三类能力在 surface 替换后的一致性与热重载行为

## 6. 辅助能力与合同补齐

- [ ] 6.1 评估是否仍需要独立“工具模糊搜索”端点；若保留，则数据源必须是当前 capability surface，而不是旧 runtime 目录
- [ ] 6.2 评估是否仍需要独立 Skill Tool；若已有 skill catalog / prompt 能力足够，则更新 spec 去掉重复设计
- [x] 6.3 保持 `application/config` 内的连接测试能力与 `server` 路由接线可用

## 7. 变更校验

- [x] 7.1 `cargo fmt --all --check`
- [x] 7.2 `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 7.3 `cargo test`
- [x] 7.4 验证架构规则：`application` 不依赖 `adapter-*`；`kernel` 不依赖 `session-runtime`；handler 只依赖 `App`
