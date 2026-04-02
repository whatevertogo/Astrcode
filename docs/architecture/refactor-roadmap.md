# Refactor Roadmap

目标：在尽量不改行为的前提下提高清晰度，冻结底层契约，为后续接入预留干净边界.

## Non-Goals

- 重写 `AgentLoop`
- 替换 session / replay 基础模型
- 引入万能 hook 系统

---

## Phase 1: Split Heavy Assembly Files — Done ✓

从 `server` 和 `runtime` 中拆出独立 crate：
- `crates/runtime-config/` — 配置模型与加载/校验/env 解析
- `crates/runtime-llm/` — LLM 提供者抽象与 OpenAI-compatible/Anthropic 适配
- `crates/runtime-prompt/` — Prompt 组装引擎与 Contributor 模式
- `crates/storage/` — JSONL 会话持久化（从 core 中提取文件系统实现）

server 入口 (`crates/server/src/main.rs`) 只消费 `astrcode-runtime` 暴露的 bootstrap surface.

## Phase 2: Capability as First-Class Action Model — Done ✓

- `Tool` trait 新增 `capability_metadata()` / `capability_descriptor()` 回调
- `ToolCapabilityInvoker` 注册到统一 `CapabilityRouter`
- built-in 和 plugin capability 进入同一路由
- `CapabilityDescriptor` 校验在装配阶段统一执行

## Phase 3: Policy and Approval Runtime Services — Done ✓

- `core::policy` — `PolicyVerdict` 三态契约 (Allow/Deny/Ask)
- `runtime::approval_service` — `ApprovalBroker` trait + `DefaultApprovalBroker`
- `AgentLoop` — 接入模型请求改写、tool call 三态分支与 broker 恢复
- `RuntimeService` 在 capability reload 时保留 `PolicyEngine` 与 `ApprovalBroker`

## Phase 4: Policy/Event Split — Done ✓

- `AgentEvent` (观测面) / `StorageEvent` (持久化面) 明确区分
- `EventTranslator` 做 StorageEvent → AgentEvent + SessionEventRecord 投影
- `PhaseTracker` 从 StorageEvent 流自动追踪 Phase 状态
- `EventLog` append-only JSONL 持久化 (`crates/storage/src/session/event_log.rs`)
- `FileSystemSessionRepository` 会话管理 (`crates/storage/src/session/repository.rs`)
- 跨进程 turn 锁 (`crates/storage/src/session/turn_lock.rs`)
- `PromptMetrics` / `CompactApplied` / `TurnDone.reason` 事件已暴露
- Token usage 统计 (`crates/runtime/src/agent_loop/token_usage.rs`)

## Phase 5: Skills and Agents Loading — Done ✓

**已完成**:
- `runtime-prompt` crate 独立拆分
- Contributor 模式: Identity / Environment / AgentsMd / CapabilityPrompt / SkillSummary / WorkflowExamples
- `AGENTS.md` / `IDENTITY.md` 分层加载
- Builtin skill 编译期自动扫描 + 运行时落盘 (`crates/runtime/build.rs`)
- `Skill` tool 通过 `CapabilityRouter` 暴露
- 两阶段 skill 加载: prompt 只暴露索引 → `Skill` tool 按需加载正文
- 用户 skill 目录: `~/.claude/skills/` + `~/.astrcode/skills/`
- 项目 skill 目录: `<working_dir>/.astrcode/skills/`
- `PromptComposer` 支持条件渲染、拓扑排序、缓存、诊断

## Phase 6: Context Compaction — Done ✓

**已完成**:
- `agent_loop/compaction.rs` — 上下文压缩管线
- `agent_loop/microcompact.rs` — 微调压缩
- `agent_loop/token_budget.rs` — Token 预算解析与续命决策（nudge 消息 + diminishing-returns 检测）
- `agent_loop/token_usage.rs` — Token 用量统计
- `AgentLoop` 已持有所有配置: `auto_compact_enabled`, `compact_threshold_percent`, `tool_result_max_bytes`, `compact_keep_recent_turns`
- `RuntimeConfig` 新增配置项: `default_token_budget`, `continuation_min_delta_tokens`, `max_continuations`
- `PromptMetrics` / `CompactApplied` 事件已暴露
- `POST /api/sessions/:id/compact` 手动压缩 API 可用
- `UserMessageOrigin::AutoContinueNudge | CompactSummary` 用于区分自动续命来源

**已接入 Policy**: `decide_context_strategy` 决策点可触发 `Compact` / `Summarize` / `Truncate` / `Ignore` 策略.

## Phase 7: Parallel Tool Execution — Done ✓

- `agent_loop/tool_cycle.rs` — 工具并行执行
- `max_tool_concurrency` 配置项已接入（env: `ASTRCODE_MAX_TOOL_CONCURRENCY`, 默认 10）
- `concurrency_safe` 工具可并行执行

## Phase 8: Turn State Machine — Done ✓

- `TurnOutcome` 枚举 (Completed/Cancelled/Error)
- `TurnDone.reason` 字段
- 移除 `max_steps` (详见 [ADR-0006](../adr/0006-turn-outcome-state-machine.md))
- `finish_turn` / `finish_with_error` / `finish_interrupted` 统一处理 turn 结束

## Phase 9: ACP / MCP Entry Points — Not Started

预留边界，不强求第一阶段完整实现.

---

## Expected End State

- Core 只定义最小契约和执行语义
- Runtime 负责装配与生命周期
- Server / CLI / ACP / Web / Tauri 都只是 adapter
- Capability 成为唯一动作模型
- Policy 成为唯一同步决策面
- Event 成为唯一异步观测面
