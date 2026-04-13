## 1. 收紧 `context` 与 request assembly 边界

- [x] 1.1 审计 `crates/session-runtime/src/context/*` 与 `crates/session-runtime/src/context_window/*`，标出仍然混有 request assembly 的实现与测试。
- [x] 1.2 在 `crates/session-runtime/src` 下建立更中性的 request assembly 子域（例如 `turn/request` 或等价模块），把最终请求拼装代码迁出 `context_window`。
- [x] 1.3 调整 `context` 与 `context_window` 的公开类型和模块注释，确保 `ResolvedContextSnapshot` 只表达来源/继承/解析结果，`context_window` 只表达预算/裁剪/压缩/窗口化。
- [x] 1.4 补齐 context/request assembly 相关单测，覆盖迁移前后的消息顺序、预算裁剪与 compaction 行为不变。

## 2. 拆分 `query` 并固定读取语义

- [x] 2.1 将 `crates/session-runtime/src/query/mod.rs` 拆分为 `history.rs`、`agent.rs`、`mailbox.rs`、`turn.rs` 四类读取子模块，并保留稳定 re-export。
- [x] 2.2 把 turn terminal snapshot、observe snapshot、recoverable parent delivery、pending delivery 等投影逻辑分别归位到对应 query 子模块。
- [x] 2.3 检查 `query` 中是否还残留推进、副作用或运行态协调代码；若存在，迁回 `actor`、`turn` 或 `application` 的正确边界。
- [x] 2.4 为四类 query 子模块补齐针对性测试，确保历史读取、agent 视图、mailbox 恢复和 turn 结果投影都可独立验证。

## 3. 收紧 `actor` / `observe` / `factory` 子域职责

- [x] 3.1 审计 `crates/session-runtime/src/actor/*`，确保其中只保留推进与 live truth 管理，不包含 observe 视图拼装或外部订阅协议映射。
- [x] 3.2 审计 `crates/session-runtime/src/observe/*`，确保其中只保留 replay/live 订阅、scope/filter、状态源整合，不新增同步快照投影算法。
- [x] 3.3 审计 `crates/session-runtime/src/factory/*`，将策略决策、校验、状态读写从 factory 中移出，只保留执行输入或执行对象构造职责。
- [x] 3.4 为 actor/observe/factory 增补必要模块注释或轻量测试，确保后续扩展时不会再次越界。

## 4. 继续收紧 `application` 到薄用例门面

- [x] 4.1 审计 `crates/application/src/agent/*` 与 `crates/application/src/execution/*`，查找仍残留的单 session 终态投影、durable append、observe 拼装细节。
- [x] 4.2 把残留的单 session 查询与命令细节继续下沉到 `SessionRuntime` 稳定接口，保留 `application` 中的参数校验、权限检查与跨 session 编排。
- [x] 4.3 回归验证 child terminal handoff、parent wake、send/observe/close 等路径，确认跨 session 协调仍留在 `application`，行为不回退。
- [x] 4.4 清理无用的旧 helper、空壳模块或重复投影代码，保持 `application` 边界清晰。

## 5. 文档与验证

- [x] 5.1 更新 `PROJECT_ARCHITECTURE.md` 中 `session-runtime` 与 `application` 的子域边界描述，写明 `context`、`context_window`、`actor`、`observe`、`query`、`factory` 的 allowed / forbidden responsibilities。
- [x] 5.2 运行并记录验证：`cargo fmt --all --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test`。
