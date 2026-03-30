# Refactor Roadmap

## Goal

这份路线图的目标不是“重写 AstrCode”，而是：

- 在尽量不改行为的前提下提高清晰度
- 冻结未来 6 到 12 个月内尽量不动的底层契约
- 为 skills、agents、approval、ACP/MCP 接入预留干净边界

## Non-Goals

当前阶段不做：

- 重写 `AgentLoop`
- 替换现有 session / replay 基础模型
- 把所有存储后端做成可随意替换的第三方插件
- 引入“万能 hook 系统”
- 把 workflow 升为 core 一等概念

## Phase 0: Freeze Contracts in Docs

先完成并确认以下设计文档：

- `01-layered-architecture.md`
- `02-core-contracts.md`
- `03-runtime-assembly.md`
- `04-events-approval-and-transports.md`

阶段目标：

- 团队对 core/runtime/transport 三层边界形成共识
- 明确哪些抽象以后尽量不改
- 先停掉“继续往大文件里加逻辑”的惯性

## Phase 1: Split Heavy Assembly Files Without Behavior Changes

首要目标是整理，不是改逻辑。

### Target

已完成：

- `crates/runtime/src/bootstrap.rs`
- `crates/runtime/src/runtime_surface_assembler.rs`
- `crates/runtime/src/runtime_governance.rs`
- `crates/runtime/src/builtin_capabilities.rs`
- `crates/runtime/src/plugin_discovery.rs`
- `crates/runtime/src/plugin_host.rs`

### Success Criteria

- 行为不变
- 测试不退化
- `server` 不再直接承载大部分 runtime 装配细节

### Status

本阶段已完成前四项，且现有 server 入口已经只消费 `astrcode-runtime` 暴露的 bootstrap / governance surface。  
仍未单独提炼的项是一个更明确的 `plugin_host.rs` 生命周期模块。

## Phase 2: Make Capability the Only First-Class Action Model

### Target

- 保留 `CapabilityRouter`
- 将 `ToolRegistry` 明确降级为 builtin capability source
- 避免新功能继续绕开 capability 模型

### Practical Changes

- 在命名和模块边界上强调 `Capability`
- 把 tool adapter 明确放到 adapter/source 模块
- 避免 runtime 直接依赖“本地 tool 列表”作为主抽象

### Success Criteria

- 新增动作能力时，默认先问“它是什么 capability”
- built-in 和 plugin capability 进入同一路由

## Phase 3: Introduce Formal Policy and Approval Runtime Services

### Target

新增运行时服务：

- `PolicyEngine`
- `ApprovalBroker`

### Notes

- 不引入万能 `PolicyHook`
- 把权限、审批、context pressure 决策点显式建模
- 审批挂起/恢复通过 broker 完成，不通过 EventBus 反向驱动

### Success Criteria

- `Allow / Deny / Ask` 成为正式 runtime contract
- transport 层只消费审批状态，不决定审批模型

## Phase 4: Introduce Runtime Observation Bus

### Target

新增 `AgentEvent` 和 `EventBus`，并与现有 `StorageEvent` 做清晰分工。

### Notes

- `AgentEvent` 面向 UI、CLI、ACP、telemetry
- `StorageEvent` 面向持久化、replay、cursor
- 两者可以投影，但不强制相等

### Success Criteria

- SSE 和其他客户端接入可以直接订阅 runtime observation
- 不再把所有瞬时 UI 事件硬塞进 durable session log

## Phase 5: Upgrade Skills and Agents Loading

### Target

把当前的能力提示从“固定摘要”升级为真正的发现与加载机制。

### Practical Changes

- `AGENTS.md` 支持更清晰的分层作用域
- `SKILL.md` 支持按需发现与按需加载
- prompt contributor pipeline 继续保留，但数据来源升级

### Success Criteria

- skills 不再只是提示块
- runtime 可以正式区分 capability source、prompt contributor、skill metadata

## Phase 6: Reserve ACP / MCP Entry Points

### Target

为后续接入：

- ACP server
- MCP bridge
- 其他外部控制器

预留稳定边界，但不强求第一阶段完整实现。

### Success Criteria

- runtime 不依赖某个具体 UI
- transport 适配只消费 runtime surface

## Suggested Validation Checklist

每个阶段结束时，至少验证：

- Rust 代码改动：`cargo fmt --all -- --check && cargo test --workspace`
- 前端代码改动：`cd frontend && npm run typecheck && npm run lint && npm run format:check`
- 如果改动 `deny.toml` 或 `Cargo.lock`：`cargo deny check bans`

## Expected End State

完成以上阶段后，AstrCode 应收敛为：

- Core 只定义最小契约和执行语义
- Runtime 负责装配与生命周期
- Server / CLI / ACP / Web / Tauri 都只是 adapter
- Capability 成为唯一动作模型
- Policy 成为唯一同步决策面
- Event 成为唯一异步观测面

这套结构既保留当前仓库已有优势，也为后续的多前端、多 provider、多插件和更严格的审批模型留下空间。
