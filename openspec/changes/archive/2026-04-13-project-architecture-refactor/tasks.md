## 重构执行原则（本清单以“最终形态”为目标）

- 不保留向后兼容层，不接受“过渡期永久化”。
- 任务顺序严格执行：先建立 `core` 契约，再抽 `kernel/session-runtime/application`，最后删除旧层。
- 未经代码验证不得勾选完成。

## 1. 重置基线与阻塞面盘点

- [x] 1.1 复核当前已勾选任务与代码现状，回退误勾选项，保证 `tasks.md` 与代码一致
- [x] 1.2 列出 `server -> runtime -> runtime-*` 真实依赖链（按 crate 与主要类型）
- [x] 1.3 列出 `RuntimeService` 被 handler 直接调用的方法清单，作为迁移验收基线

## 2. core 基座重建（能力语义 + 端口契约 + 强类型）

- [x] 2.1 在 `core` 定义 `CapabilitySpec`，并补齐 `CapabilityKind`、`InvocationMode`、`SideEffect`、`Stability`、`PermissionSpec`
- [x] 2.2 收敛强类型 ID / 名称模型：`SessionId`、`TurnId`、`AgentId`、`CapabilityName`
- [x] 2.3 在 `core` 新建端口模块并定义稳定契约：`EventStore`、`LlmProvider`、`PromptProvider`、`ResourceProvider` 及所需 provider/gateway trait
- [x] 2.4 将 `ToolCapabilityMetadata`、`Tool`、`CapabilityInvoker` 改为返回 `CapabilitySpec`
- [x] 2.5 将 policy / plugin / registry 等运行时语义判断改为消费 `CapabilitySpec`
- [x] 2.6 将稳定配置结构从 `runtime-config` 迁入 `core/config`：`Config`、`ConfigOverlay`、`Profile`、`ModelConfig`、`RuntimeConfig`、`AgentConfig`、`ActiveSelection`、`CurrentModelSelection`、`ModelOption`
- [x] 2.7 从 `core` 移除 `astrcode-protocol` 依赖并更新导出
- [x] 2.8 验证：`cargo check -p astrcode-core`

## 3. protocol 收口为 wire DTO

- [x] 3.1 保留 `CapabilityDescriptor` 作为传输 DTO，不参与运行时内部语义判断
- [x] 3.2 新建 `CapabilitySpec <-> CapabilityDescriptor` mapper，统一边界转换
- [x] 3.3 将 server DTO、插件握手、协议输出统一收口到 mapper
- [x] 3.4 验证：`cargo check -p astrcode-protocol`

## 4. adapter 契约化与命名收敛

- [x] 4.1 完成实现层命名收敛：`storage -> adapter-storage`、`runtime-llm -> adapter-llm`、`runtime-prompt -> adapter-prompt`、`runtime-mcp -> adapter-mcp`、`runtime-skill-loader -> adapter-skills`、`runtime-agent-loader -> adapter-agents`
- [x] 4.2 合并 `runtime-tool-loader + runtime-agent-tool -> adapter-tools`
- [x] 4.3 统一更新 workspace `members`、各 crate `Cargo.toml` 与源码路径
- [x] 4.4 约束 adapter 只实现 `core` 端口，移除对 `kernel/session-runtime/application/server/runtime*` 的反向依赖
- [x] 4.5 将 `adapter-tools`、`adapter-prompt`、`adapter-mcp`、`plugin` 的能力语义统一切到 `CapabilitySpec`
- [x] 4.6 验证：`cargo check --workspace`

## 5. 抽取 kernel（只保留全局控制面）

- [x] 5.1 新建 `crates/kernel`，依赖限制为 `core + 第三方库`
- [x] 5.2 将 `runtime-registry` 的 `CapabilityRouter`、`CapabilityRouterBuilder`、`ToolCapabilityInvoker` 迁入 `kernel/registry`
- [x] 5.3 将 `runtime-agent-control` 迁入 `kernel/agent_tree`，去除对 `runtime-config` 的耦合
- [x] 5.4 将全局 `loop_surface` 状态与刷新协调迁入 `kernel/surface`
- [x] 5.5 在 `kernel/gateway` 提供统一入口：`invoke_tool`、`call_llm`、`build_prompt`、`read_resource`
- [x] 5.6 在 `kernel/events` 实现 `EventHub`（仅全局事件）
- [x] 5.7 明确禁止将 `build_agent_loop`、`LoopRuntimeDeps` 迁入 kernel
- [x] 5.8 定义 `Kernel`、`KernelBuilder`、`KernelError`，公共 API 不暴露内部容器与锁
- [x] 5.9 验证：`cargo check -p astrcode-kernel`

## 6. 抽取 session-runtime（唯一会话真相面）

- [x] 6.1 新建 `crates/session-runtime`，仅依赖 `core + kernel`
- [x] 6.2 将 `runtime-session` 状态模型迁入 `session-runtime/state`
- [x] 6.3 将 `runtime/service/session/*` 的 create/load/delete/list/catalog 广播迁入 `session-runtime/catalog`
- [x] 6.4 将 `runtime-agent-loop`（含 `build_agent_loop`、`LoopRuntimeDeps`、`AgentLoop`、`TurnRunner`、context-window/compaction）迁入 `session-runtime/turn` 或 `session-runtime/factory`
- [x] 6.5 将 `runtime-execution`（`PreparedAgentExecution`、`ResolvedContextSnapshot`）与 `runtime/service/execution/*` 迁入 `session-runtime/actor`、`session-runtime/context`
- [x] 6.6 将 `runtime/service/turn/*`、`runtime/service/agent/*` 迁入 `session-runtime` 对应模块
- [x] 6.7 将 `SubAgentExecutor`、`CollaborationExecutor` 实际桥接实现迁入 `session-runtime`
- [x] 6.8 定义 `SessionRuntime { sessions, event_store, kernel, catalog_events }` 与 `SessionActor`，并确保 Actor 不直接持有 provider
- [x] 6.9 公共 API 统一使用 `SessionId`、`TurnId`、`AgentId`，不传裸 `String`
- [x] 6.10 验证：`cargo check -p astrcode-session-runtime`

## 7. 抽取 application（唯一用例边界）并重建治理模型

- [x] 7.1 新建 `crates/application`，仅依赖 `core + kernel + session-runtime`
- [x] 7.2 将 `runtime/service/config/*`、`composer/*`、`lifecycle/*`、`watch/*`、`mcp/*`、`observability/*` 迁入 `application` 对应模块
- [x] 7.3 将 `runtime/service/service_contract.rs` 重建为 `application/errors.rs` 与稳定服务契约
- [x] 7.4 将配置 IO / 路径解析 / 默认值 / 环境变量解析 / 校验逻辑迁入 `application/config`
- [x] 7.5 定义 `App { kernel, session_runtime }`，实现参数校验、权限前置检查、业务错误归类
- [x] 7.6 迁移 `RuntimeGovernance`、`RuntimeCoordinator`、`RuntimeHandle` 职责，重建 `AppGovernance`（或等价命名）治理模型
- [x] 7.7 将 active plugins/capabilities 快照、reload 结果、shutdown 协调统一纳入新治理模型
- [x] 7.8 更新状态快照与 reload 相关类型归属到 `application`，不再留在旧 `runtime` 门面
- [x] 7.9 验证：`cargo check -p astrcode-application`

## 8. server 组合根收口与 handler 解耦

- [x] 8.1 在 `crates/server/src/bootstrap/runtime.rs` 建立唯一业务组合根
- [x] 8.2 将旧 `runtime/src/bootstrap.rs` 的组装逻辑迁入 server bootstrap
- [x] 8.3 在组合根中显式组装 adapter、`Kernel`、`SessionRuntime`、`App`、`AppGovernance`
- [x] 8.4 更新 `server/main.rs`、`server/src/bootstrap/mod.rs`、`AppState`，使 handler 仅依赖 `application`
- [x] 8.5 替换 handler 对 `RuntimeService` 及服务句柄的直接调用，改为 `App`/`application` 服务接口
- [x] 8.6 更新状态接口、mapper、测试，统一改用 `AppGovernance` 快照类型
- [x] 8.7 确保 HTTP 状态码映射仅发生在 server 层
- [x] 8.8 验证：`cargo check -p astrcode-server`

## 9. 删除旧 runtime 体系与遗留 crate

- [x] 9.1 检查runtime有无遗漏移植的然后再删除 `crates/runtime`
- [x] 9.2 检查runtime-config有无遗漏移植的然后再删除 `crates/runtime-config`
- [x] 9.3 检查runtime-session有无遗漏移植的然后再删除 `crates/runtime-session`
- [x] 9.4 检查runtime-execution有无遗漏移植的然后再删除 `crates/runtime-execution`
- [x] 9.5 检查runtime-agent-loop有无遗漏移植的然后再删除 `crates/runtime-agent-loop`
- [x] 9.6 检查runtime-agent-control有无遗漏移植的然后再删除 `crates/runtime-agent-control`
- [x] 9.7 检查runtime-registry有无遗漏移植的然后再删除 `crates/runtime-registry`
- [x] 9.8 检查有无遗漏移植的然后再删除被替代的旧实现层 crate（若仍残留）
- [x] 9.9 检查有无遗漏移植的然后再更新 workspace 根 `Cargo.toml`，清理所有旧成员与无效依赖
- [x] 9.10 检查有无遗漏移植的然后再代码检索确认不再存在 `RuntimeGovernance`、`RuntimeCoordinator`、`RuntimeHandle`、`RuntimeService` 的旧路径依赖

## 10. 文档与最终验证

- [x] 10.1 更新 `PROJECT_ARCHITECTURE.md`，反映最终依赖图与分层边界
- [x] 10.2 更新必要的开发文档（含模块迁移路径与命名约定）
- [x] 10.3 执行 `cargo fmt --all --check`
- [x] 10.4 执行 `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 10.5 执行 `cargo test`
