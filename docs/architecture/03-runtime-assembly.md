# Runtime Assembly

## Goal

Layer 2 负责把 Layer 1 的抽象变成一个真正可运行的 agent runtime。

它做的不是“定义平台事实”，而是“把默认实现装起来”。

## Responsibilities

Runtime assembly 需要承担以下职责：

- capability 注册与命名冲突检查
- provider / tool / storage / compaction 的默认实现装配
- 内置插件与外部插件的加载、初始化、健康检查、reload
- policy chain 的构造
- approval service 的构造
- event bus 的构造
- session store 与 replay source 的构造
- `AGENTS.md` / `SKILL.md` / layered config 的发现与加载

## Recommended Runtime Components

建议 Layer 2 至少显式拥有以下组件：

- `PluginHost`
- `CapabilityRegistry`
- `CapabilityRouter`
- `PolicyEngine`
- `ApprovalBroker`
- `EventBus`
- `SessionStore`
- `ProviderAdapter`
- `SkillsLoader`
- `AgentsLoader`
- `McpBridge`
- `RuntimeBootstrap`

## Built-in vs External Plugins

建议继续保持双轨：

### Built-in Plugins

特点：

- Rust trait object
- 编译期链接
- 适合 provider、基础文件工具、默认 store、默认 compaction

适合的内容：

- `provider.openai`
- `provider.anthropic`
- `tool.fs.read`
- `tool.fs.edit`
- `tool.shell.exec`
- `store.jsonl`
- `policy.permission_basic`
- `strategy.compaction_basic`

### External Plugins

特点：

- 进程隔离
- 通过协议桥接
- 适合社区扩展和用户自定义能力

适合的内容：

- 外部工具生态
- MCP capability bridge
- 实验性 agent workflow
- 组织自定义能力

## Why PluginHost Belongs Here

`PluginHost` 是运行时装配机制，不是核心语义。

它需要处理：

- `discover()`
- `init()`
- `reload()`
- `health_check()`
- `crash_isolation()`
- `shutdown()`

这些事情都不应该污染 Layer 1。

## Current Code Assessment

当前仓库里最需要整理的不是 loop，而是 runtime assembly 的代码位置。

最典型的例子是：

- `crates/runtime/src/bootstrap.rs`
- `crates/runtime/src/runtime_surface_assembler.rs`
- `crates/runtime/src/runtime_governance.rs`
- `crates/runtime/src/builtin_capabilities.rs`

这组模块当前承担了：

- runtime bootstrap
- plugin manifest discovery
- plugin initialization
- built-in tool registration
- capability conflict detection
- governance snapshot / reload
- plugin health probing

相比之前，这些职责已经从 `server` 成功下沉到 `runtime` crate。

## Recommended Split

当前仓库已经落地为：

- `bootstrap.rs`
- `runtime_surface_assembler.rs`
- `runtime_governance.rs`
- `builtin_capabilities.rs`
- `plugin_discovery.rs`
- `approval_service.rs`

其中：

- `bootstrap.rs` 负责“把一个 runtime 装起来”
- `runtime_surface_assembler.rs` 负责“把 built-in + plugin 组装成统一 capability surface”
- `runtime_governance.rs` 负责 reload / health / snapshot
- `builtin_capabilities.rs` 负责默认 capability source
- `plugin_discovery.rs` 负责插件搜索路径与 manifest 发现
- `approval_service.rs` 负责默认审批 broker 及其生命周期边界

runtime assembly 还承担一项很重要但容易被忽略的职责：  
在 capability 真正进入统一 router 之前，对插件上报的 descriptor 做宿主侧校验。这样插件作者即便绕过 builder 直接构造 descriptor，错误也会在装配阶段被显式拒收。

对内置工具也一样。  
`ToolCapabilityInvoker` 不再硬编码统一的 `builtin / Workspace / Stable` 元数据，而是优先读取 `Tool::capability_metadata()` 或 `Tool::capability_descriptor()`。这样权限 hint、side-effect 和稳定性会跟着工具实现本身演进，而不是散落在 adapter 层。

Phase 3 之后，`RuntimeService` 也会显式持有 `PolicyEngine` 与 `ApprovalBroker`，并在 capability surface reload 时保留这两个运行时服务，而不是偷偷回退成默认行为。

## Capability Router Stays the Center

`CapabilityRouter` 应该继续是运行时装配层的中心。

这和已接受的 [ADR-0003](../adr/0003-unified-capability-routing.md) 一致：

- built-in tool 先适配成 `CapabilityInvoker` 再注册到 router
- plugin capability 走 invoker 接入 router
- runtime 只消费 router

因此不建议回退到“tool registry 是主抽象”的模型。

## Prompt and Skill Loading

`PromptComposer` 和 contributors 是当前代码里值得保留的模式。

建议继续保留并加强：

- `AGENTS.md` 分层加载
- `SKILL.md` 按需发现与按需注入
- profile-specific prompt block
- runtime 级别的 few-shot 和 guardrail block

相关代码：

- `crates/runtime/src/prompt/contributors/agents_md.rs`
- `crates/runtime/src/prompt/contributors/skill_summary.rs`

未来要做的不是推翻这套机制，而是把它从“固定摘要块”升级为“真正的 loader + contributor pipeline”。

## Runtime Bootstrap Contract

建议把 runtime bootstrap 收敛为一个小而稳定的入口：

```rust
async fn bootstrap(config: RuntimeConfig) -> Result<RuntimeSurface>;

struct RuntimeSurface {
    agent_loop: AgentLoop,
    router: CapabilityRouter,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalBroker>,
    events: Arc<dyn EventBus>,
    sessions: Arc<dyn SessionStore>,
    governance: Arc<RuntimeGovernance>,
}
```

`server` 只消费 `RuntimeSurface`，不应该继续自己拼装半套 runtime 细节。
