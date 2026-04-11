# Collaboration Runtime Checklist: Astrcode Agent 协作四工具重构

**Purpose**: 审查 008 规格是否把四工具公开面、生命周期、durable mailbox 与观测边界写得完整、清晰且可验收
**Created**: 2026-04-12
**Feature**: [spec.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/spec.md)

## Requirement Completeness

- [ ] CHK001 是否明确规定了 `spawn`、`send`、`observe`、`close` 四个公开协作工具各自的职责边界，且没有遗漏根 Agent 与子 Agent 的双向协作场景？ [Completeness, Spec §FR-001, Spec §FR-003, Spec §FR-006]
- [ ] CHK002 是否为“子 Agent 完成单轮后继续存活”的关键状态迁移写清了进入条件、退出条件和唯一终止路径？ [Completeness, Spec §FR-004, Spec §FR-005, Spec §FR-015]
- [ ] CHK003 是否对 mailbox durable 化需要记录的最小字段集写全，包括 `delivery_id`、发送方快照、目标身份和可打开会话目标？ [Completeness, Spec §FR-009, Spec §FR-010, Spec §FR-012]
- [ ] CHK004 是否明确规定了 `observe` 必须返回的所有快照字段，且这些字段足以支持父 Agent 做继续 `send` 或 `close` 的决策？ [Completeness, Spec §FR-013]
- [ ] CHK005 是否写明了 legacy shared-session 数据的“只读保留、禁止新写”边界，避免实现阶段继续偷偷生成旧结构？ [Completeness, Spec §FR-016, Edge Cases]

## Requirement Clarity

- [ ] CHK006 “可继续接收消息的空闲状态”是否被量化为清晰可判定的生命周期语义，而不是模糊表述为“看起来已完成”？ [Clarity, Spec §FR-004, Spec §FR-005]
- [ ] CHK007 “下一个可用轮次被唤醒处理”是否定义得足够清楚，能够区分“立即插入当前轮”与“延迟到下一轮”的行为差异？ [Clarity, Spec §FR-007, Spec §FR-008]
- [ ] CHK008 规格是否把“稳定 `delivery_id` 允许因恢复而重复出现”讲清楚，避免调用方把重放误解为全新任务？ [Clarity, Spec §FR-009, Spec §FR-011, Spec §FR-019]
- [ ] CHK009 “observe 只允许直接父级访问”是否写清楚了直接父级、非直接父级、兄弟节点和跨树节点四类身份边界？ [Clarity, Spec §FR-014, Acceptance Scenario 3.3]

## Requirement Consistency

- [ ] CHK010 关于“只有 `close` 才会终止 Agent”的要求，是否与用户故事、边界场景和假设章节保持一致，没有残留“自动结束”或“resume 公开可用”的冲突描述？ [Consistency, Spec §FR-015, Assumptions]
- [ ] CHK011 关于“子 Agent 结束后返回 Idle”与“发送到已终止 Agent 必须拒绝”的规则，是否在所有场景中保持一致，没有把失败/取消误写成终态？ [Consistency, Spec §FR-004, Spec §FR-006, Edge Cases]
- [ ] CHK012 关于 mailbox 的 `at-least-once` 语义、snapshot drain 边界和去重策略，是否在功能要求与成功标准之间保持一致，没有出现 exactly-once 暗示？ [Consistency, Spec §FR-008, Spec §FR-011, Spec §SC-003, Spec §SC-004]

## Acceptance Criteria Quality

- [ ] CHK013 成功标准是否足够客观，能够独立验证“四工具公开面只剩四个”而不是依赖主观判断“看起来简化了”？ [Measurability, Spec §SC-001]
- [ ] CHK014 成功标准是否可直接验证“子 Agent 无需重建即可执行第二条指令”，而不是只验证第一次 `spawn` 成功？ [Acceptance Criteria, Spec §SC-002]
- [ ] CHK015 成功标准是否把“静默丢失”与“显式关闭后丢弃”区分清楚，确保恢复语义能被客观判定？ [Acceptance Criteria, Spec §SC-003]
- [ ] CHK016 成功标准是否足以验证 `observe` 返回的是完整快照而不是部分字段拼装结果？ [Acceptance Criteria, Spec §SC-005, Spec §FR-013]

## Scenario Coverage

- [ ] CHK017 是否同时覆盖了主流程、忙碌时排队、重启恢复、父空闲被子唤醒和非授权观测五类核心场景？ [Coverage, User Story 1, User Story 2, User Story 3]
- [ ] CHK018 是否明确覆盖了“轮开始后收到新消息”的并发边界，避免实现者把新消息混入当前轮上下文？ [Coverage, Spec §FR-008, Edge Cases]
- [ ] CHK019 是否覆盖了“子 Agent 发送消息后立即被关闭”这一恢复型边界状态，并说明父级稍后看到该消息为何仍属合法？ [Coverage, Edge Cases]

## Dependencies & Assumptions

- [ ] CHK020 是否把“根 Agent 进入同一控制树”“继续复用现有 session event log”“不提供旧公开兼容入口”这些关键假设写成了可追踪约束，而不是隐含前提？ [Dependencies, Assumption, Spec §FR-003, Assumptions]
- [ ] CHK021 是否明确记录了提示词/工具描述也属于 008 的交付范围，而不是默认只有 runtime 代码需要迁移？ [Completeness, Spec §FR-018, Spec §SC-001]

## Ambiguities & Conflicts

- [ ] CHK022 规格是否仍存在“最近输出摘要”“活动任务摘要”“待处理任务摘要”等字段定义粒度不一致的问题，需要进一步量化数据来源或截断规则？ [Ambiguity, Spec §FR-013]
- [ ] CHK023 是否需要补充“close 幂等性”或“关闭不存在 Agent 的返回约定”，以避免 server/API 层实现各自发挥？ [Gap]

## Notes

- 这份清单用于复查 008 需求文本是否足够支撑实现和验收，不直接判定代码是否正确。
- 若后续继续迭代 008，可在同一文件追加新条目并延续编号。
