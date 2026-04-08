# Feature Specification: 子 Agent 独立子会话模型

**Feature Branch**: `003-subagent-child-sessions`  
**Created**: 2026-04-08  
**Status**: Draft  
**Input**: User description: "Refactor subagent architecture so child agents run as durable child sessions with independent transcripts, tool-based communication, wait/resume/close control, parent-child handoff summaries, and a UI model that shows thinking, tool activity, and final replies while preparing the runtime model for future fork agents."

## User Scenarios & Testing

### User Story 1 - 稳定委派与交付 (Priority: P1)

作为主 agent 的使用者和维护者，我希望子 agent 在被创建后成为独立的子会话，即使父 turn 已经结束，子 agent 仍能继续工作，并在完成时向主 agent 交付稳定、可追溯的最终回复。

**Why this priority**: 这是当前问题的根因修复点。只有先把“子 agent 不是父 turn 附属执行流”建立起来，后续的恢复、查看、打回和 UI 改善才有可靠基础。

**Independent Test**: 创建一个会在父 turn 结束后继续运行的子 agent；验证其不会被自动取消，并且在结束后主 agent 能收到一次且仅一次终态交付。

**Acceptance Scenarios**:

1. **Given** 主 agent 启动了一个长时间运行的子 agent，**When** 父 turn 正常结束，**Then** 子 agent 继续保持运行状态，直到显式取消或进入终态。
2. **Given** 子 agent 已完成自己的工作，**When** 系统生成交付结果，**Then** 主 agent 收到包含子 agent 最终回复、状态和可查看引用的结构化交付，而不是零散事件片段。
3. **Given** 子 agent 失败或被终止，**When** 主 agent 收到终态通知，**Then** 通知中包含失败原因或最后有效进展，避免出现“只看到探索看不到结论”的情况。

---

### User Story 2 - 可查看的子会话视图 (Priority: P1)

作为维护者，我希望每个子 agent 都有独立、可重放、可打开的子会话视图，这样我可以从父会话进入子会话，查看它的思考摘要、工具活动和最终答复，而不是阅读原始协议 JSON。

**Why this priority**: 即使生命周期修复了，如果子 agent 的内容仍然只能混在父流中显示，维护者仍然难以理解系统行为，也难以定位问题。

**Independent Test**: 运行至少一个成功的子 agent 和一个失败的子 agent；从父会话进入各自的子会话视图，验证都能看到清晰的时间线和最终状态。

**Acceptance Scenarios**:

1. **Given** 父会话中存在多个子 agent，**When** 用户选择其中一个子 agent，**Then** 系统打开对应的子会话视图，并显示该子 agent 的思考摘要、工具活动和最终答复。
2. **Given** 子 agent 已经结束且父会话被重新加载，**When** 用户再次打开该子 agent，**Then** 仍然能看到完整的子会话历史和终态信息。
3. **Given** 默认会话视图，**When** 子 agent 结果在父会话中呈现，**Then** 只展示结构化摘要和交付信息，原始协议 JSON 默认隐藏。

---

### User Story 3 - 主子双向协作 (Priority: P2)

作为主 agent，我希望能继续向一个正在运行、已完成但可恢复、或需要返工的子 agent 发送补充要求，让它继续完善结果，而不是每次都重新创建新的子 agent。

**Why this priority**: 没有持续通信能力，子 agent 只能“一次性跑完”，无法形成真正的协作回路，也无法实现你讨论中的“打回去重新完善”。

**Independent Test**: 创建一个子 agent，先让它完成初版答复，再由主 agent 向其发送修订要求，验证同一个子 agent 会话继续产出新的终态交付。

**Acceptance Scenarios**:

1. **Given** 子 agent 正在运行，**When** 主 agent 发送补充说明或修订要求，**Then** 子 agent 接收到新的输入并继续在原子会话中工作。
2. **Given** 子 agent 已经完成但未被关闭，**When** 主 agent 要求其补充证据或改写答案，**Then** 系统恢复同一个子 agent 会话，而不是强制创建新实例。
3. **Given** 主 agent 需要等待某个子 agent 的结果，**When** 调用等待能力，**Then** 系统只等待目标子 agent，不影响其他子 agent 或父会话的正常工作。
4. **Given** 主 agent 决定停止某个子 agent，**When** 调用关闭或取消能力，**Then** 只有目标子 agent 进入终态，不会误伤同批次的其他子 agent。

---

### User Story 4 - 为未来 Fork Agent 复用同一底座 (Priority: P3)

作为架构维护者，我希望未来的 fork agent 不是另一套并行系统，而是复用同一个子会话、状态机和交付模型，只在“创建来源”上与普通子 agent 不同。

**Why this priority**: 如果现在不把 lineage 和创建模式纳入统一模型，后续加入 fork agent 时很可能再次引入第三套生命周期和展示语义。

**Independent Test**: 审查子会话模型后，能明确区分“新任务创建的子会话”和“继承父上下文创建的子会话”，且二者共享相同的查看、等待、恢复、关闭和交付规则。

**Acceptance Scenarios**:

1. **Given** 一个普通子 agent 会话和一个继承上下文的子会话，**When** 系统记录它们的来源信息，**Then** 两者都能通过同一种生命周期和查看规则被管理。
2. **Given** 未来新增 fork 创建入口，**When** 维护者接入该入口，**Then** 无需重新定义子会话展示、通信和终态交付协议。

### Edge Cases

- 父 turn 结束、父会话刷新或父 UI 关闭时，仍在运行的子 agent 不应被隐式取消。
- 子 agent 在没有生成最终文本前失败、超时或被关闭时，系统仍需提供终态原因和最后有效进展。
- 多个子 agent 由同一个父 turn 同时启动时，等待、关闭、恢复和通知必须按目标子 agent 精确作用。
- 主 agent 向已处于终态的子 agent 发送新要求时，系统需要明确恢复原会话，而不是悄悄丢弃消息或新建重复实例。
- 子 agent 的最后一个有效输出如果不是完整答复，系统需要选择最后可读的交付文本，避免父会话只收到空结果。
- 会话回放、重载或崩溃恢复时，子 agent 不能丢失自己的终态、回复内容或父子关系。

## Requirements

### Functional Requirements

- **FR-001**: 系统 MUST 在每次创建子 agent 时生成独立的子会话标识，该标识独立于父 turn 和父工具调用。
- **FR-002**: 系统 MUST 将子 agent 的状态、历史、最终答复和关联产物作为独立子会话持久化，而不是仅作为父会话中的临时事件流保存。
- **FR-003**: 父 turn 的正常完成 MUST NOT 自动取消仍在运行的子 agent；只有显式关闭、取消、会话销毁策略或子 agent 自身终态才能结束其生命周期。
- **FR-004**: 系统 MUST 为每个子 agent 保存父会话引用、来源 turn、创建来源类型和当前状态，以支持后续查看、恢复和调试。
- **FR-005**: 主 agent MUST 能通过工具能力创建子 agent，并获得一个可用于后续通信、等待、恢复和关闭的稳定引用。
- **FR-006**: 主 agent MUST 能向目标子 agent 发送后续输入，以补充要求、要求返工或继续同一任务。
- **FR-007**: 系统 MUST 支持针对指定子 agent 的等待能力，并在目标子 agent 进入终态后返回其终态状态和最新交付摘要。
- **FR-008**: 系统 MUST 支持针对指定子 agent 的关闭或取消能力，且该操作不得影响未被选中的其他子 agent。
- **FR-009**: 已完成但未被关闭的子 agent MUST 保持可恢复状态，允许主 agent 在原会话内继续追加任务或修订要求。
- **FR-010**: 当子 agent 进入终态时，系统 MUST 向父会话生成结构化交付摘要，至少包含子 agent 身份、终态状态、最终答复或失败原因、以及进入子会话的查看入口。
- **FR-011**: 默认父会话视图 MUST 以摘要形式展示子 agent 的思考、工具活动和最终答复，MUST NOT 默认展示原始协议 JSON。
- **FR-012**: 独立子会话视图 MUST 展示子 agent 的思考摘要、工具活动、最终答复和终态信息，并支持在会话重载后重新打开。
- **FR-013**: 系统 MUST 在子 agent 无法产出完整最终答复时，尽可能保留最后有效的可读结果并连同失败原因一起交付给父会话。
- **FR-014**: 会话重放与恢复 MUST 重建子 agent 的可查看状态、终态状态和父子关系，避免出现终态丢失、重复通知或回复缺失。
- **FR-015**: 系统 MUST 区分“新任务创建的子会话”和“继承上下文创建的子会话”两类来源，并让两类来源共享同一生命周期、通信和查看模型。
- **FR-016**: 系统 MUST 将主会话与子会话之间的通信顺序持久化，确保补充要求、子 agent 响应和终态通知在回放时顺序一致。

### Key Entities

- **父会话**: 发起子 agent、接收交付摘要、并保留子会话引用的主协作上下文。
- **子会话**: 承载单个子 agent 独立生命周期、历史、状态和最终答复的协作实体。
- **子 agent 引用**: 主 agent 用于后续发送输入、等待、恢复或关闭目标子会话的稳定标识。
- **交付摘要**: 子 agent 进入终态后回传给父会话的结构化结果，包含状态、最终答复或失败原因，以及子会话查看入口。
- **通信记录**: 主会话与子会话之间追加输入、接收结果和终态通知的顺序化交互记录。
- **创建来源**: 记录子会话是基于全新任务创建还是基于继承上下文创建的 lineage 元信息，用于支持未来 fork agent。

## Success Criteria

### Measurable Outcomes

- **SC-001**: 在覆盖“父 turn 先结束、子 agent 后完成”的验证场景中，100% 的子 agent 能继续运行至自身终态，不会因父 turn 完成而被提前结束。
- **SC-002**: 在成功、失败、取消三类终态场景中，100% 的子 agent 都会向父会话生成一次且仅一次结构化交付摘要。
- **SC-003**: 在包含至少 10 次子 agent 创建、补充要求、等待、恢复和关闭的验证流程中，目标操作的命中率达到 100%，不存在误操作到其他子 agent 的情况。
- **SC-004**: 在会话重载或恢复后的验证场景中，100% 的已存在子会话都能重新打开并显示其最终答复或失败原因。
- **SC-005**: 默认父会话和子会话界面在所有验证场景下都隐藏原始协议 JSON，仅展示结构化的思考摘要、工具活动和最终答复。
- **SC-006**: 在“子 agent 初次答复后被主 agent 打回完善”的验证场景中，95% 以上的案例能在同一子会话内完成二次交付，而无需创建新的重复子 agent。

## Assumptions

- 本特性的范围是建立统一的子会话生命周期、持久化和交互模型；完整的 fork agent 用户入口不在本次交付范围内，但必须被该模型直接支持。
- 现有历史会话可以保留旧格式读取策略，但新创建的子 agent 一律遵循新的独立子会话模型。
- 原始协议事件或调试元数据可以继续保留用于诊断，但默认用户界面不直接展示这些内容。
- 子会话在同一父会话 lineage 下保持唯一归属，即使被多次恢复，也不会改变其父会话来源。
