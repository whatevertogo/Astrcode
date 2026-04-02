# ADR-0004: Freeze Layered Core, Runtime, and Transport Boundaries

- Status: Accepted
- Date: 2026-03-30

## Context

AstrCode 当前的主要问题已经不再是“能力不够”，而是部分装配代码和核心语义开始混杂：

- `AgentLoop`、capability router、prompt contributors 已经初步形成稳定内核
- runtime 装配、plugin lifecycle、health / reload 逻辑分散在 `server` 与其他模块之间
- transport 层仍然容易承载过多业务装配职责

如果不及时冻结分层边界，后续继续接入：

- approval
- skills / agents layering
- ACP / CLI / MCP
- 多 provider / 多 capability source

时，核心语义和 transport / runtime 细节会继续互相污染。

## Decision

冻结 AstrCode 的高层分层为三层：

### 1. Layer 1: Immutable Core Contracts

Layer 1 只保留以下平台核心契约：

- `AgentLoop` Contract
- `Capability` Contract
- `Policy` Contract
- `Event` Contract

Layer 1 不包含：

- `PluginHost`
- 具体 provider / tool / storage 实现
- HTTP / SSE / Axum 细节
- CLI / ACP / Tauri / Web 适配

### 2. Layer 2: Runtime Assembly

Layer 2 负责把 core contract 组装为可运行 runtime，包括：

- capability registry / router 装配
- plugin discovery / init / reload / health
- policy engine
- approval broker
- event bus
- session store
- skills / agents / layered config loading

当前 Layer 2 已拆分为多个独立 crate，保持编译隔离：

- `crates/runtime/` — 运行时门面（`RuntimeService`、`AgentLoop`、bootstrap、governance）
- `crates/storage/` — JSONL 会话持久化（`EventLog`、`FileSystemSessionRepository`）
- `crates/runtime-config/` — 配置模型与加载/校验
- `crates/runtime-llm/` — LLM 提供者抽象与 OpenAI/Anthropic 适配
- `crates/runtime-prompt/` — Prompt 组装引擎与 Contributor 模式
- `crates/plugin/` — 插件宿主（supervisor、peer、loader、transport）
- `crates/tools/` — 内置工具实现（fs、shell 等）

### 3. Layer 3: Transports and External Adapters

Layer 3 负责对外暴露 runtime，包括：

- HTTP / SSE server
- Tauri / Web
- CLI
- ACP
- MCP bridge

`server is truth` 仍然是 AstrCode 的产品架构原则，但 `server` 在代码分层上属于 transport / adapter 层，而不是核心语义定义层。

## Consequences

正面影响：

- 后续功能演进优先发生在 runtime assembly 或 transport 层
- core 契约可以更稳定
- transport 不再天然拥有业务装配权
- PluginHost 不再被误视为核心语义

代价：

- 当前部分位于 `server` 的装配代码需要下沉或拆分
- runtime 层会显式承担更多框架化职责
- 新模块边界需要补充测试和文档以维持清晰度
