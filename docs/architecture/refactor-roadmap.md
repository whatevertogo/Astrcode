# Refactor Roadmap

目标：在尽量不改行为的前提下提高清晰度，冻结底层契约，为后续接入预留干净边界。

## Non-Goals

- 重写 `AgentLoop`
- 替换 session / replay 基础模型
- 引入万能 hook 系统

---

## Phase 1: Split Heavy Assembly Files — Done

从 `server` 和 `runtime` 中拆出独立 crate：

- `crates/runtime-config/` — 配置模型与加载/校验
- `crates/runtime-llm/` — LLM 提供者抽象与 OpenAI/Anthropic 适配
- `crates/runtime-prompt/` — Prompt 组装引擎与 Contributor 模式
- `crates/storage/` — JSONL 会话持久化（从 core 中提取文件系统实现）

server 入口只消费 `astrcode-runtime` 暴露的 bootstrap / governance surface。

## Phase 2: Capability as First-Class Action Model — Done

- `Tool` trait 新增 `capability_metadata()` / `capability_descriptor()` 回调
- `ToolCapabilityInvoker` 注册到统一 `CapabilityRouter`
- `HandlerDispatcher` 等冗余适配层已清理
- built-in 和 plugin capability 进入同一路由

## Phase 3: Policy and Approval Runtime Services — Done

- `core::policy` 已冻结为 `PolicyVerdict` 三态契约（Allow/Deny/Ask）
- `runtime::approval_service` 已提供默认 `ApprovalBroker`
- `AgentLoop` 已接入模型请求改写、tool call 三态分支与 broker 恢复
- `RuntimeService` 在 capability reload 时保留 `PolicyEngine` 与 `ApprovalBroker`

待后续：`ContextStrategyDecision` 接入真正 token budgeting / compaction 触发路径。

## Phase 4: Runtime Observation Bus — Partial

已完成：
- `AgentEvent` / `StorageEvent` 已明确区分（`core/src/event/domain.rs` 与 `types.rs`）
- `EventTranslator` 做 StorageEvent → AgentEvent 投影（`core/src/event/translate.rs`）
- `EventLog` append-only JSONL 持久化已实现（`storage/src/session/event_log.rs`）
- `FileSystemSessionRepository` 已实现（`storage/src/session/repository.rs`）

待完成：
- `EventBus` 作为 runtime 级别 broadcast 机制（当前 AgentEvent 通过 SSE 直接推送，无独立 bus）
- `ApprovalRequested` / `ApprovalResolved` 作为独立事件发射

## Phase 5: Skills and Agents Loading — Partial

已完成：
- `runtime-prompt` crate 独立拆分，Contributor 模式组装系统提示
- 已有 Contributor：Identity / AgentsMd / Environment / SkillSummary
- `AGENTS.md` 分层加载已支持

待完成：
- `SKILL.md` 按需发现与按需加载
- 区分 capability source / prompt contributor / skill metadata

## Phase 6: ACP / MCP Entry Points — Not Started

预留边界，不强求第一阶段完整实现。

---

## Expected End State

- Core 只定义最小契约和执行语义
- Runtime 负责装配与生命周期
- Server / CLI / ACP / Web / Tauri 都只是 adapter
- Capability 成为唯一动作模型
- Policy 成为唯一同步决策面
- Event 成为唯一异步观测面
