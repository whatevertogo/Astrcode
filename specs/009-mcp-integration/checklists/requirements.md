# Specification Quality Checklist: MCP Server 接入支持

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-04-12
**Updated**: 2026-04-12 (第二轮：融合 Claude Code 设计分析 + 用户反馈)
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs) — 注：`CapabilityInvoker`、`CapabilityRouter` 是项目已有的概念，非新技术选型
- [x] Focused on user value and business needs
- [x] Written for non-technical stakeholders
- [x] All mandatory sections completed
- [x] Clarifications section documents all design decisions

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria are technology-agnostic (no implementation details)
- [x] All acceptance scenarios are defined (5 个用户故事 × 2-5 个验收场景)
- [x] Edge cases are identified (10 个边界条件)
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] No implementation details leak into specification

## Validation Details

### 从 Claude Code 源码引入的设计要点

- 连接状态机（connected/failed/needs-auth/pending/disabled）
- 工具命名规范 `mcp__{server}__{tool}` 避免冲突
- 工具 annotations（readOnly/destructive/openWorld）映射到能力元数据
- 指数退避重连（1s → 30s, max 5 次，仅远程传输）
- 本地/远程差异化并发策略（3 vs 10）
- 配置作用域分层（user/project/local）+ 签名去重
- 项目级审批流程
- 策略允许/拒绝列表
- List change notification 响应式更新
- Instructions 增量注入 prompt 组装管线
- 输出复用已有落盘机制而非重新发明

### 用户反馈整合

- stdio 安全信任模型 → Assumptions 新增
- 工具冲突可观测性 → FR-025
- 远程认证（headers + OAuth） → FR-016/FR-017/FR-018
- SC-001 拆分系统延迟/端到端延迟 → 已更新
- 取消 + 强制断开 → FR-013
- 热加载 vs 进行中调用 → Edge Cases 新增
- 复用 TOOL_RESULT_INLINE_LIMIT → FR-023

## Notes

- 所有检查项均通过
- FR-018（OAuth）、FR-020（prompts）、FR-021（resources）使用 SHOULD，作为增强特性
- 假设部分明确说明了安全信任模型和 v1 不支持的功能（sampling、stdio 自动重启）
- Edge cases 从 5 个扩展到 10 个，覆盖了配置去重、环境变量缺失、list_changed 通知等场景
- 功能需求从 14 条扩展到 28 条，按主题分组便于理解
