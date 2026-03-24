# ADR-0002: Freeze Coding Profile Context Boundary

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 的长期定位是“面向编码场景的智能体平台”，但协议层不能被设计成只能处理代码编辑的专用协议。

如果把编码场景的字段直接写死在顶层消息里，会导致：

- 协议骨架失去通用性
- 后续引入 `general-agent` 或 `workflow` profile 时需要破坏顶层结构
- UI、runtime、plugin 的临时字段不断污染协议边界

因此需要把“通用调用骨架”和“coding 专业语义”彻底分层。

## Decision

冻结 AstrCode Protocol V4 的上下文边界为：

- `InvocationContext` 负责通用调用上下文
- `profile_context` 负责 profile 专属语义
- `coding` 是首个官方 profile

### 1. 顶层 `InvocationContext` 只保留通用字段

通用字段包括：

- `request_id`
- `trace_id`
- `session_id`
- `caller`
- `workspace`
- `deadline_ms`
- `budget`
- `profile`
- `metadata`

这些字段允许被通用 agent、workflow runtime 和 coding runtime 共同使用。

### 2. Coding 语义冻结在 `profile_context`

`coding` profile 当前冻结的首批字段为：

- `workingDir`
- `repoRoot`
- `openFiles`
- `activeFile`
- `selection`
- `approvalMode`

这些字段属于协议的一部分，但不升级为通用顶层字段。

### 3. Workspace 语义分两层表达

- `workspace` 表达跨 profile 都可能需要的仓库或工作区引用
- `profile_context` 表达 coding runtime 当下真正需要的 IDE / editor 细节

### 4. 新增 coding 字段走 profile 版本演进

后续如果需要增加：

- `outputMode`
- `focusedSymbol`
- `diffBase`
- `reviewMode`

等 coding 专属字段，优先通过 `coding` profile 扩展，而不是修改通用顶层壳。

## Consequences

正面影响：

- AstrCode 可以继续保持 coding-first
- 协议基础层仍然保留向通用 agent 演进的空间
- UI 专属临时字段更难渗透进公共协议层

代价：

- 插件作者需要理解“通用上下文”和“coding profile 上下文”的区别
- SDK 需要提供更好的 helper，避免作者直接手搓 `profile_context`
