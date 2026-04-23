# Plan Mode 工作流

Plan mode 是 Astrcode 内置的三种治理模式之一（`code`、`plan`、`review`(未完善)），通过模式切换和计划工件（plan artifact）实现"先规划后执行"的工作流。

## 整体流程

```
用户发起任务
    │
    ▼
LLM 判断需要规划
    │
    ▼  enterPlanMode / /mode plan
ModeChanged: code -> plan
    │
    ▼  注入 plan mode prompt + plan template
LLM 阅读代码 ──> 起草计划 ──> 自审 ──> 修订
    │                                      │
    │              ◄────────────────────────┘
    │                       (循环直到计划可执行)
    ▼
exitPlanMode（第一次）
    │  结构校验 + 内审 checkpoint
    ▼  返回 review pending
exitPlanMode（第二次）
    │  status -> awaiting_approval
    ▼  ModeChanged: plan -> code
计划展示给用户
    │
    ▼  用户输入 "同意" / "approved"
status -> approved
    │  创建归档快照
    ▼  注入 plan exit declaration
LLM 进入 code mode，按计划执行
```

## 模式定义

### 能力面约束

Plan mode 通过 `CapabilitySelector` 收缩工具面（`crates/application/src/mode/catalog.rs`）：

```
可见工具 = AllTools
           - SideEffect(Local | Workspace | External)
           - Tag("agent")
           + Name("exitPlanMode")
           + Name("upsertSessionPlan")
```

即：**只暴露只读工具 + 两个计划专属工具**，禁止文件写入、外部调用和 agent 委派。

### 治理策略

| 属性 | 值 |
|------|-----|
| 子 session 委派 | 禁止（`allow_delegation: false`） |
| 子 session 约束 | restricted |
| Turn 并发策略 | `RejectOnBusy`（拒绝并发，不分支） |
| 可切换目标 | `code`、`plan`、`review` 均可 |

## 计划工件

### 存储路径

```
~/.astrcode/projects/<project>/sessions/<session-id>/plan/
    <slug>.md        # 计划内容（Markdown）
    state.json       # 计划状态元数据
```

归档快照存储在：
```
~/.astrcode/projects/<project>/plan-archives/<timestamp>-<slug>/
    plan.md          # 归档的计划 Markdown
    metadata.json    # 归档元数据
```

### 状态模型

`SessionPlanState`（`crates/core/src/session_plan.rs`）：

```rust
pub enum SessionPlanStatus {
    Draft,              // 草稿：LLM 编辑中
    AwaitingApproval,   // 等待用户审批
    Approved,           // 已批准：开始执行
    Completed,          // 已完成
    Superseded,         // 被新计划取代
}
```

每个 session 有且仅有一个 canonical plan（单一真相）。同一任务反复修订覆盖同一 plan；用户切换任务时，LLM 覆盖旧 plan。

### 计划模板

计划 Markdown 必须遵循以下结构（`crates/application/src/mode/builtin_prompts/plan_template.md`）：

```markdown
# Plan: <title>

## Context
(背景与当前状态)

## Goal
(目标)

## Scope
(范围)

## Non-Goals
(不做的事)

## Existing Code To Reuse
(可复用的现有代码)

## Implementation Steps
(具体实施步骤)

## Verification
(验证方法)

## Open Questions
(待确认问题)
```

退出 plan mode 时，系统会校验以下必要章节必须存在：

| 章节 | 必须存在 | 必须包含可执行项 |
|------|---------|----------------|
| `## Context` | 是 | 否 |
| `## Goal` | 是 | 否 |
| `## Existing Code To Reuse` | 是 | 否 |
| `## Implementation Steps` | 是 | 是 |
| `## Verification` | 是 | 是 |

"可执行项"指章节内容必须包含以 `- `、`* ` 或数字开头的行。

## 两个计划专属工具

### `upsertSessionPlan`

创建或更新计划工件。参数：`{ title, content, status? }`。

- 从 title 推导 slug（小写、连字符分隔、最长 48 字符）
- 写入 `<slug>.md` 和 `state.json`
- 每次写入重置 `reviewed_plan_digest`（触发重新审核循环）

### `exitPlanMode`

退出 plan mode 的审批门控，包含两阶段校验：

**第一阶段：结构校验**

检查计划 Markdown 包含所有必要章节且关键章节包含可执行项。校验失败返回 `sessionPlanExitReviewPending`（kind: `revise_plan`），列出缺失章节或无效章节。

**第二阶段：内审 checkpoint**

计算计划内容的 FNV-1a-64 摘要。如果摘要与 `state.reviewed_plan_digest` 不同：

1. 保存摘要到 state
2. 返回 `sessionPlanExitReviewPending`（kind: `final_review`）
3. LLM 必须再次调用 `exitPlanMode`

摘要相同（第二次调用）时：

1. 设置 `status = AwaitingApproval`
2. 触发 `ModeChanged { from: plan, to: code }` 事件
3. 返回 `sessionPlanExit`（包含完整计划内容）

这个两阶段设计确保 LLM 在展示计划给用户之前至少自审了一次。

## Prompt 注入

Plan mode 通过 prompt program 注入四组声明（`crates/application/src/mode/builtin_prompts/`）：

### Plan Mode Prompt（`plan_mode.md`）

注入时机：进入 plan mode 后的每一 turn。

核心约束：
- 必须先阅读代码再起草计划
- 必须通过 `upsertSessionPlan` 写入计划
- 不允许执行任何实现工作
- 必须在计划可执行后才调用 `exitPlanMode`
- 退出前必须进行内部自审
- 退出后必须向用户总结计划并请求批准
- 不允许静默切换到执行模式

### Plan Re-entry Prompt（`plan_mode_reentry.md`）

注入时机：session 已有 plan artifact 且重新进入 plan mode。

指导 LLM 先读取当前计划，同一任务则修订，不同任务则覆盖。

### Plan Exit Prompt（`plan_mode_exit.md`）

注入时机：用户批准计划后，LLM 回到 code mode。

```
The session has exited plan mode and is now back in code mode.

Execution contract:
- Use the approved session plan artifact as the primary implementation reference.
- The user approval already happened; do not ask for plan approval again.
- Start implementation immediately unless the user message clearly requests more planning.
```

### Plan Template（`plan_template.md`）

注入时机：plan mode 首次进入且无现有计划。

提供计划 Markdown 的骨架模板。

## 用户审批

用户审批是文本匹配机制（`crates/application/src/session_use_cases.rs`），当 `status == AwaitingApproval` 时检查用户消息：

| 语言 | 批准关键词 |
|------|-----------|
| 英文 | `approved`、`go ahead`、`implement it` |
| 中文 | `同意`、`可以`、`按这个做`、`开始实现` |

匹配后系统：
1. 调用 `mark_active_session_plan_approved()`（status -> `Approved`）
2. 如果仍处于 plan mode，自动切换到 code mode
3. 创建计划归档快照（`write_plan_archive_snapshot`）
4. 注入 `build_plan_exit_declaration` 到 prompt

没有显式"拒绝"操作——用户直接给出修改意见，LLM 继续留在 plan mode 修订计划。

## 模式切换机制

### 进入 plan mode

三种途径：

1. **LLM 主动切换**：在 code mode 中调用 `enterPlanMode({ reason: "..." })` 工具
2. **用户命令**：`/mode plan` slash 命令，走统一治理入口校验
3. **自动切换**：审批边界自动切换（不常见）

所有路径都通过 `validate_mode_transition()` 校验，然后触发 `ModeChanged` 事件。

### 退出 plan mode

- `exitPlanMode` 工具：`plan -> code`，需要通过两阶段校验
- 用户审批后自动切换

### 事件持久化

`ModeChanged` 事件（`crates/core/src/event/types.rs`）记录到 JSONL event log：

```rust
ModeChanged {
    from: ModeId,
    to: ModeId,
    timestamp: DateTime<Utc>,
}
```

`AgentStateProjector` 在 apply 时更新 `mode_id` 字段。旧 session 不含此事件时回退到默认 mode。

## 设计要点

### 为什么需要两阶段 exitPlanMode

第一次调用是结构校验 + 强制自审 checkpoint。LLM 可能在自审中发现计划不完善并修订。第二次调用确认计划内容未变（摘要匹配），才真正提交给用户。这避免了 LLM 直接把粗略计划甩给用户的情况。

### 为什么不用显式审批事件

计划生命周期通过 `state.json` 和工具返回元数据（`sessionPlanExit`、`sessionPlanExitReviewPending`）追踪，不需要独立的 `PlanCreated`/`PlanApproved` 存储事件。审批语义已由 `ModeChanged` + `upsertSessionPlan` 的状态变更覆盖。

### 为什么限制为单一 canonical plan

避免多计划导致的混乱和选择困难。同一任务保持单一 plan，迭代修订。切换任务时覆盖，简洁明确。
