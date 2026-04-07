# Compact 系统规范

## 1. 范围

本文档定义 Astrcode Compact 系统的：

- 当前已落地主线
- 摘要与压缩边界的稳定要求
- 后续增强的优先顺序与约束

## 2. 当前已实现基线

根据现有实现与文档整理，当前已具备：

- 电路熔断
- post-compact 附件恢复
- auto-continue nudge
- anchored token budget
- 增量重压缩
- 413 降级重试
- 微压缩（microcompact）
- compaction tail snapshot

这些能力构成当前 compact 主线，后续增强不应破坏它们。

## 3. 设计目标

1. 压缩后仍然能继续执行当前任务。
2. 压缩边界能被 replay、UI 和调试链路理解。
3. 压缩过程可审计、可扩展、可逐步增强。
4. 性能优化不应改变 session 真相与内容语义。

## 4. 核心约束

### 4.1 Compact 是上下文边界，不是普通消息摘要

compact 的结果会改变后续 prompt 组装边界，因此系统必须同时保留：

- compact summary
- recent tail
- 必要的附件恢复能力
- `CompactApplied` 事件

### 4.2 摘要内容要求

compact summary 至少应覆盖：

- 当前目标
- 关键约束与偏好
- 已完成进展
- 关键决策与原因
- 明确的下一步
- 必要的恢复上下文

### 4.3 compact 不应引入新语义面

以下能力若实现，只能作为优化：

- cache-sharing fork
- context edit
- 时间触发微压缩
- prompt cache 命中优化

它们不应修改 session 真相、事件协议或 UI 基础语义。

## 5. 优先增强方向

### 5.1 Compact Hook

系统应允许在 compact 前后挂接扩展点，用于：

- 增补系统提示
- 调整保留范围
- 提供自定义摘要
- 执行 compact 后恢复逻辑

但 hook 语义必须保持收敛，避免把 compact 变成一个自由脚本系统。

### 5.2 Prompt 工程

compact prompt 应强调：

- 无工具执行
- 结构化输出
- 保留 why / constraints / decisions
- 提升 scannable 程度
- 支持增量重压缩

### 5.3 可审计 pruning

当需要丢弃旧工具结果时，应优先使用 prune 标记或占位，而不是直接无痕删除。

### 5.4 更精确的 token 计数

当前启发式可继续使用，但中期应补更精确的 token 计量与预算分层保护。

## 6. 后续增强方向

以下能力属于中后期：

- file operation tracking
- ghost snapshot / undo 保护
- mid-turn compact
- context usage 可视化
- split turn 处理
- session memory

这些能力在引入前必须先说明：

- 它改变的是语义还是只是性能
- 是否影响 replay / audit / UI
- 是否需要新的事件或 API

## 7. 非目标

- 不把 compact 做成另一套独立会话协议
- 不为了 prompt 美观牺牲恢复能力
- 不在当前阶段引入专用 compact agent 作为前置要求

## 8. 对应文档

- 设计入口：[../design/compact-system-design.md](../design/compact-system-design.md)
- 开放项：[./open-items.md](./open-items.md)
