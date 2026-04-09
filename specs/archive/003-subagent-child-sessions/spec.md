# Feature Specification: 子 Agent 子会话边界与缓存优化

**Feature Branch**: `003-subagent-child-sessions`  
**Created**: 2026-04-09  
**Status**: Draft  
**Input**: User description: "将子智能体架构优化为独立子会话模型，并补齐存储分离、共享渲染缓存、结构化上下文继承、父子唤醒通知、lineage 索引与 resume 语义，使父子 Agent 的 durable 边界、缓存契约和恢复行为稳定且可验证。"

## Scope Overview

本特性将父子 Agent 协作模型收敛为四个明确边界：

1. **会话身份边界**: 一个 Agent 对应一个 Session；每次 spawn 创建新的子 Session；resume 复用原 Session，只创建新的执行实例。
2. **持久化边界**: 父 Session 只保留子任务的边界事件与摘要投影；子 Session 保留自己的完整事件历史。
3. **缓存边界**: 父子在同一运行时内共享安全的渲染缓存，但不得因错误 key 或消息污染导致错误复用。
4. **恢复与唤醒边界**: resume 必须从子 Session 历史恢复；子结果唤醒父 Agent 时不得破坏父消息缓存连续性。

## User Scenarios & Testing

### User Story 1 - 独立子会话成为 durable 真相 (Priority: P1)

作为维护者，我希望每个子 Agent 从创建开始就拥有独立 Session 和独立历史，这样父会话与子会话的职责清晰、恢复路径稳定、排障时无需从一份混写日志里反推真实边界。

**Why this priority**: 如果 Session 身份与 durable 边界不先定死，后续的 lineage、resume、缓存和 UI 视图都会建立在不可靠前提上。

**Independent Test**: 从一个父会话连续创建多个子 Agent；验证每个子 Agent 都拥有独立 Session 标识和独立持久化历史，且父日志只包含边界事件，不包含子内部细节流。

**Acceptance Scenarios**:

1. **Given** 父 Agent 发起一次新的子任务委派，**When** 系统创建子 Agent，**Then** 系统为该子 Agent 分配一个新的子 Session 身份，并记录本次执行实例标识。
2. **Given** 同一父会话连续启动多个子 Agent，**When** 这些子 Agent 并行或串行运行，**Then** 每个子 Agent 都写入自己的独立持久化历史，互不混写。
3. **Given** 父会话查看自己的 durable 历史，**When** 某个子 Agent 在运行中调用工具、产生中间摘要或完成回复，**Then** 父会话只看到子任务边界事件、摘要投影和终态交付，不看到子内部完整事件流。
4. **Given** 旧历史数据仍包含共享写入模式，**When** 系统读取历史会话，**Then** 系统继续支持旧数据读取与回放，但所有新创建的子 Agent 都采用独立子 Session 模型。

---

### User Story 2 - Resume 复用原 Session 而不是重开新会话 (Priority: P1)

作为维护者，我希望子 Agent 被恢复时复用原子会话身份，并从已有历史重建状态继续工作，而不是重开一个看似相似但身份不同的新会话。

**Why this priority**: resume 语义是 durable 架构的试金石。只要 resume 退化成 respawn，lineage、缓存、上下文与 UI 入口都会同时失真。

**Independent Test**: 让一个子 Agent 运行到中途后停止，再对同一子 Agent 执行 resume；验证 Session 身份不变、执行实例更新、恢复后继续基于原历史运行。

**Acceptance Scenarios**:

1. **Given** 某个子 Agent 已经存在并保留完整历史，**When** 系统对该子 Agent 执行 resume，**Then** 系统复用原子 Session 身份，并为本次恢复生成新的执行实例标识。
2. **Given** 某个子 Agent 在恢复前已有用户消息、系统摘要和工具活动，**When** resume 开始，**Then** 系统从该子 Session 的持久化历史重建恢复状态，而不是以空状态重新启动。
3. **Given** 父会话依赖该子 Agent 的稳定入口，**When** 子 Agent 被恢复，**Then** 父会话继续通过同一子 Session 入口观察和管理它，并收到一条明确的已恢复通知。
4. **Given** 系统因缺失 lineage 或历史损坏而无法安全恢复子 Agent，**When** 用户或父 Agent 发起恢复，**Then** 系统必须明确暴露恢复失败，而不是悄悄创建新的并列会话替代原子会话。

---

### User Story 3 - 父子共享安全缓存且继承上下文结构化传递 (Priority: P1)

作为维护者，我希望同一运行时中的父子 Agent 能安全复用稳定 prompt 渲染结果，并把父背景信息以结构化方式传给子 Agent，这样重复 spawn 子 Agent 时能降低 token 浪费，同时不牺牲正确性。

**Why this priority**: 这是当前性能浪费和上下文粗糙传递的共同根因。如果仍把背景文本拼进第一条用户消息，缓存命中和任务语义都会持续变差。

**Independent Test**: 在相同工作目录、配置和工具范围下连续创建多个相似子 Agent；验证稳定背景渲染被复用，子 Agent 的任务消息只包含任务本身，父背景作为独立继承块出现。

**Acceptance Scenarios**:

1. **Given** 父 Agent 与子 Agent 运行在同一运行时实例中，且稳定背景条件一致，**When** 子 Agent 启动，**Then** 系统复用已渲染的稳定背景内容，减少重复渲染成本。
2. **Given** 两次子 Agent 启动在工作目录、配置、工具范围或规则内容上存在差异，**When** 系统判断缓存是否可复用，**Then** 系统只在安全条件满足时复用，绝不因简化 key 而错误命中。
3. **Given** 父会话存在 compact summary 与最近活动，**When** 子 Agent 启动，**Then** 父背景作为结构化继承块注入到子 Agent 的系统上下文中，任务消息本身只保留任务与直接上下文说明。
4. **Given** 最近活动中包含大量工具输出或长文本，**When** 系统构建传给子 Agent 的 recent tail，**Then** 系统优先保留语义关键项与摘要，不把无界原文或噪音整体继承给子 Agent。
5. **Given** compact summary 与 recent tail 的变化频率不同，**When** 系统构建继承上下文，**Then** 两者作为独立继承块参与缓存与更新，避免高频变化内容拖累低频稳定内容的复用率。

---

### User Story 4 - 子结果唤醒父 Agent 时不污染父消息历史 (Priority: P2)

作为维护者，我希望子 Agent 完成后既能可靠唤醒父 Agent 继续工作，又不会把变化的交付详情塞进父消息历史，导致父会话缓存连续性被破坏。

**Why this priority**: 子任务交付是协作闭环的最后一环。如果交付机制本身持续制造消息缓存 miss，系统会在多子 Agent 协作场景下反复为同一稳定前缀付费。

**Independent Test**: 让多个子 Agent 在父 turn 结束后先后交付；验证父 Agent 被成功唤醒处理每次交付，同时父会话历史不出现携带可变交付详情的用户消息。

**Acceptance Scenarios**:

1. **Given** 子 Agent 在父 turn 结束后完成工作，**When** 系统将子结果送回父 Agent，**Then** 父 Agent 被唤醒继续工作，且父消息历史不因这次交付而引入变化的详情消息。
2. **Given** 子 Agent 交付包含摘要、终态和引用信息，**When** 父 Agent 下一次构建工作上下文，**Then** 这些交付详情以一次性结构化输入参与本次构建，并在消费后清除。
3. **Given** 进程重启或本轮唤醒前的运行时暂存丢失，**When** 系统重新读取 durable 历史，**Then** 父会话仍能通过边界事件和子 Session 本身还原交付事实，而不是把一次性输入队列当作 durable 真相。
4. **Given** 多个子 Agent 几乎同时交付，**When** 父 Agent 被依次唤醒处理，**Then** 每次交付都只影响其对应的父侧处理，不会导致消息串扰、重复消费或跨子任务污染。

### Edge Cases

- 同一父会话在短时间内并发创建多个子 Agent 时，父 durable 历史仍只包含边界事件，不得混入某个子 Agent 的内部工具流。
- 子 Agent 恢复时如果历史存在但 lineage 信息缺失或冲突，系统必须明确标记为不可安全恢复，而不是推断一个可能错误的父子关系。
- 两次 prompt 背景内容不同但长度相同的场景，系统不得把“长度相同”误判为“内容相同”而复用错误缓存。
- 父背景 recent tail 中包含超长工具输出时，系统必须提炼为语义摘要；不得因去掉 200 字截断而改成无界全文继承。
- 子 Agent 完成时若父 Agent 当前空闲，需要可靠唤醒；若父 Agent 正在忙于其他工作，也不得丢失该交付。
- 运行时实时观测信息在进程重启后可以消失，但 durable 真相、lineage 和子会话入口必须保留。
- 如果一次性父侧交付输入在构建前丢失，系统仍需依赖 durable 边界事件与子会话内容保证可追溯，不得出现“交付事实不存在”的假象。

## Requirements

### Functional Requirements

- **FR-001**: 系统 MUST 将每个 Agent 绑定到唯一 Session 身份；每次 spawn MUST 创建新的子 Session 身份，而不是复用已有子 Session。
- **FR-002**: 系统 MUST 将执行实例身份与 Session 身份区分开；同一子 Session 可以拥有多次执行实例，用于首次运行与后续 resume。
- **FR-003**: 系统 MUST 将新创建子 Agent 的完整内部历史持久化到其自己的子 Session 中，包括消息、工具活动、摘要、终态和恢复所需元数据。
- **FR-004**: 父 Session 的 durable 历史 MUST 只记录子任务边界事件、父侧可消费通知和终态交付摘要，MUST NOT 混入子 Session 的完整内部事件流。
- **FR-005**: 系统 MUST 将父对子的实时观测定义为运行时能力；运行中步骤、当前工具、实时 token 消耗和是否存活等信息可由内存态与事件广播提供，但 MUST NOT 作为父 durable 真相的唯一来源。
- **FR-006**: 系统 MUST 为每个新子 Session 记录可验证的父子关系信息，至少包含父 Session 身份、子 Session 身份和当前执行实例标识。
- **FR-007**: 系统 MUST 维护独立的 lineage 索引，以支持从父 Session 查询直接子 Session、从子 Session 查询父 Session、以及从执行实例查询所属子 Session。
- **FR-008**: lineage 索引 MUST 以 durable 记录为真相来源；当父侧边界记录与子侧元数据冲突时，系统 MUST 标记不一致并显式降级，而不是隐式伪造 ancestry。
- **FR-009**: resume 子 Agent 时，系统 MUST 复用原子 Session 身份，并从该子 Session 的 durable 历史重建恢复状态。
- **FR-010**: resume 成功时，系统 MUST 生成新的执行实例标识，并向父侧写入明确的“已恢复”通知。
- **FR-011**: 如果系统无法从 durable 历史安全重建某个子 Session，系统 MUST 返回明确失败，不得以空状态或新 Session 替代原 resume 请求。
- **FR-012**: 新创建的子 Session MUST 默认采用独立持久化模式；旧的共享写入模式仅用于历史读取与回放兼容，不再作为新数据的默认行为。
- **FR-013**: 在同一运行时实例内，系统 MUST 允许父子 Agent 共享稳定 prompt 渲染缓存，以减少重复子 Agent 启动时的稳定背景重渲染成本。
- **FR-014**: 缓存复用判断 MUST 基于稳定且可验证的上下文指纹，至少覆盖规范化工作目录、活动配置、排序后的工具允许集、规则文件内容摘要和 contributor 版本。
- **FR-015**: 系统 MUST NOT 仅使用长度、数量或其他会导致内容碰撞的弱特征作为缓存正确性的唯一判断依据。
- **FR-016**: 当上下文指纹不一致时，系统 MUST 放弃复用相关缓存，而不是继续命中旧结果。
- **FR-017**: 系统 MUST 将父传子的背景信息与子任务本身分离表示；子任务消息仅描述任务与直接上下文，不包含完整父摘要或最近活动拼接文本。
- **FR-018**: 系统 MUST 将父的 compact summary 与 recent tail 作为独立继承块传给子 Agent，使两类信息能独立更新与独立参与缓存。
- **FR-019**: 系统 MUST 对 recent tail 应用语义筛选与摘要规则，优先保留关键用户输入、关键助手结论与必要工具结果摘要。
- **FR-020**: 系统 MUST 对大型工具输出、长文本和低价值噪音进行摘要或裁剪，避免无界继承导致子 Agent 上下文膨胀。
- **FR-021**: 子 Agent 的首条任务消息 MUST 能被单独理解为“要做什么”，而不是混合“要做什么”和“父历史全文”。
- **FR-022**: 当子 Agent 进入终态时，系统 MUST 向父侧生成结构化交付，至少包含子 Session 身份、执行实例标识、终态状态、可读摘要和查看入口。
- **FR-023**: 系统 MUST 能在父 turn 已结束的情况下重新唤醒父 Agent 处理新的子交付。
- **FR-024**: 父 Agent 的唤醒机制 MUST 避免把变化的交付详情直接写成父消息历史中的普通任务消息，以保护父侧稳定消息前缀的连续性。
- **FR-025**: 子交付详情 MUST 以一次性结构化输入参与父 Agent 的下一次上下文构建，并在被消费后清除。
- **FR-026**: 一次性结构化输入队列 MUST 只承担运行时桥接职责，不作为 durable 真相参与历史回放或崩溃恢复。
- **FR-027**: 即使一次性结构化输入在进程重启后丢失，系统仍 MUST 通过 durable 边界事件和子 Session 本身保留交付事实与排障能力。
- **FR-028**: 父侧默认视图 MUST 以摘要投影形式展示子 Agent 的状态、最近进展和最终结论；完整内部时间线仅在目标子 Session 中查看。
- **FR-029**: 子 Session 入口、终态和可读时间线 MUST 独立于父侧视图的折叠与展开状态持久化，确保刷新与重载后仍可稳定进入。
- **FR-030**: 在多个子 Agent 并发交付、恢复或运行时，系统 MUST 保证每次唤醒、摘要投影和 lineage 查询都只作用于目标子任务，不出现串扰、误关联或重复消费。

### Key Entities

- **父 Session**: 发起子任务、接收边界事件与摘要投影的主会话实体。
- **子 Session**: 承载单个子 Agent durable 真相、内部历史、执行边界和恢复元数据的独立会话实体。
- **执行实例**: 某个 Session 的一次运行实例，用于区分首次运行与后续 resume。
- **边界事件**: 父侧 durable 历史中保留的子任务启动、完成、失败、恢复和交付事件。
- **lineage 索引**: 提供父 Session、子 Session 与执行实例之间映射关系的独立查询模型。
- **继承上下文块**: 父传子的结构化背景信息，由 compact summary 与 recent tail 等独立部分组成。
- **共享渲染缓存**: 在同一运行时实例内可被父子 Agent 安全复用的稳定 prompt 渲染结果。
- **父侧摘要投影**: 父 Session 中供继续决策和查看的子任务简化表示，包含状态、摘要和入口。
- **一次性交付输入**: 仅服务父 Agent 下一次上下文构建的运行时结构化输入，不参与 durable 历史回放。

## Success Criteria

### Measurable Outcomes

- **SC-001**: 在连续创建 10 个以上子 Agent 的验证场景中，100% 的新子 Agent 都产生独立子 Session，且父 durable 历史中不出现子内部工具事件或最终答复正文。
- **SC-002**: 在至少 10 个 resume 验证场景中，100% 的 resume 都复用原子 Session 身份并生成新的执行实例；不存在 resume 退化为新建并列 Session 的情况。
- **SC-003**: 在同一工作目录、相同配置和相同工具范围下重复启动相似子 Agent 的场景中，重复启动的稳定背景渲染开销较首次启动下降至少 70%。
- **SC-004**: 在“内容不同但长度相同”的缓存安全测试中，100% 的测试都不会出现错误缓存命中或错误 prompt 复用。
- **SC-005**: 在父背景继承验证场景中，100% 的子 Agent 首条任务消息都只包含任务与直接上下文，不包含父 compact summary 或 recent tail 的拼接全文。
- **SC-006**: 在包含大文本工具输出的 recent tail 验证场景中，100% 的继承上下文都以语义摘要形式传递，不出现无界全文继承。
- **SC-007**: 在“父 turn 已结束、多个子 Agent 随后交付”的验证场景中，100% 的子交付都能触发父 Agent 继续处理，且父消息历史不出现携带可变交付详情的普通任务消息。
- **SC-008**: 在重启恢复验证场景中，100% 的父子关系查询都能通过 lineage 索引和 durable 历史还原到正确子 Session；若数据不一致，系统会显式暴露而不是静默伪造。

## Assumptions

- 本轮规格聚焦于子 Session 的 durable 边界、缓存与恢复语义，不扩展新的用户级协作工具集合。
- 旧的共享写入数据需要继续可读，但不会为其补做历史迁移；新创建的子任务统一进入独立子 Session 模型。
- 父侧对运行中子 Agent 的实时观测以运行时能力为主；进程重启后允许实时观测丢失，但不允许 durable 真相丢失。
- 父子 Agent 共享的仅是安全可复用的渲染缓存，不包括跨请求直接复用模型供应商的请求级缓存。
