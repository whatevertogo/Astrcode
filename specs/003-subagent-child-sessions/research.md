# Research: 子 Agent Child Session 与协作工具重构

## Decision 1: 主子协作统一暴露为 tool 契约，但 runtime 内部使用定向投递层实现

**Decision**  
模型侧看到的主子协作全部统一成 tool 能力，至少包括 `spawn / send / wait / close / resume / deliver`；但 runtime 内部不把这些协作再递归实现成完整 tool 调用链，而是通过面向目标 agent 的 inbox / mailbox / notification 投递层完成送达、唤醒、恢复与去重。

**Rationale**  
tool surface 统一后，prompt 学习成本、权限治理、日志审计和后续 fork agent 扩展都更稳定；而把内部投递和 tool 执行分层，可以避免每次父子通信都重新走一遍 tool cycle，减少延迟、重复事件和恢复复杂度。

**Alternatives considered**

- 所有内部协作都直接递归成 tool 执行：会造成双重事件流、重复消费和 UI 噪音。
- 完全不用 tool，直接暴露内部 inbox 给模型：会让 prompt surface、权限模型和审计语义失控。

## Decision 2: 子 agent 的 durable 真相是独立 child session + agent ownership tree，而不是父 turn

**Decision**  
每个子 agent 都建模为独立 child session 和独立 agent 节点，持久化 `parent_session_id / parent_agent_id / parent_turn_id / lineage_kind / agent_id / child_session_id / status` 这组 ownership 真相。父 turn 只负责触发，不再是 child 生命周期 owner。

**Rationale**  
这正是当前“父 turn 结束后 child 也跟着乱掉”“看得到探索看不到最终回复”的根因。只要 child 生命周期仍挂在父 turn 上，恢复、重放、等待和继续协作都会继续绕着 turn 边界打转。

**Alternatives considered**

- 继续把 subrun 当成父 turn 的附属执行：无法支撑持续协作、恢复和 future fork。
- 只保留 live registry，不写 durable ownership：重启、reload 或重放后 child session 语义会丢失。

## Decision 3: 父会话只消费 child summary / notification，不吞 child 原始事件流

**Decision**  
父会话 durable history 中只保留对子会话可消费的通知、摘要和终态投影；child session 自己保留完整 transcript、thinking、tool activity 和最终回复。父视图从这些 notification 生成 summary card，打开 child session 时再直接加载 child session 的 history/events。

**Rationale**  
这能同时解决“视图丑、信息噪音大、原始 JSON 泄漏给 UI、父会话里看不到结论”四个问题，也符合 Claude Code / Codex 这类工具把主会话与子线程内容分流的设计。

**Alternatives considered**

- 把 child 全量消息继续混入 parent session：父视图难读，恢复和去重都复杂。
- parent 只保留一个最终字符串：会丢失 child session 可打开性与阶段性协作能力。

## Decision 4: 存储路径嵌套只能是实现优化，ownership 真相不能依赖“subsessions 文件夹”

**Decision**  
child session 的领域真相以稳定 session id + lineage metadata 为准，而不是以“是否物理嵌套在父 session 文件夹下”决定。若后续为了本地浏览或缓存在父目录下增加 `children/`、`subsessions/`、`artifacts/` 等 sidecar 结构，它们只能是索引或缓存，不能成为 lookup 与 ownership 的唯一事实源。

**Rationale**  
宪法已经明确要求 Ownership Over Storage Mode。把真相绑到物理路径层，会让 session 查找、跨边界恢复、fork 和兼容旧历史都变得更脆弱。

**Alternatives considered**

- 直接把 child session 真正存到父 session 目录下并依赖目录关系解析 ownership：查找复杂，且把实现细节变成领域事实。
- 完全不允许 parent-local sidecar：会限制本地浏览和缓存优化空间，没必要。

## Decision 5: 协作控制面要从“spawn + cancel”扩展为完整工具族，CapabilityRouter 作为唯一 runtime 注册中心

**Decision**  
当前 `spawnAgent` 与 `cancelSubRun` 的半套控制面要扩展成完整协作工具族；这些工具最终都注册为 `CapabilityInvoker`，由 `CapabilityRouter` 统一持有。`ToolRegistry` 仅保留为测试和装配辅助，不再承担生产执行主抽象。

**Rationale**  
现有 runtime 已经基本把 `CapabilityRouter` 作为主入口，但 `ToolRegistry` 仍在测试和部分装配里保留了双轨语义。child collaboration tool 数量一旦增加，这种双轨会立刻变成维护负担。

**Alternatives considered**

- 继续在生产路径同时维护 ToolRegistry 和 CapabilityRouter：重复注册与上下文转换会越来越重。
- 新建第三套 agent 协作注册中心：与当前“收口到单一入口”的方向冲突。

## Decision 6: 前端从 `SubRunThreadTree` 迁移到“父摘要列表 + 子会话直开”

**Decision**  
前端不再把父会话消息和子执行消息混合构造成 `SubRunThreadTree` 作为唯一 child 浏览模型，而是把 parent summary 列表、active child session、child breadcrumb 这三层视图显式拆开：父层只消费 summary notification，子层直接 `loadSession(child_session_id)` 读取完整 transcript。

**Rationale**  
当前 `buildSubRunThreadTree` 是在 mixed-session 假设下工作的，它适合 subrun thread，不适合独立 child session/inbox 模型。继续沿用会让后端明明已经拆开了 child session，前端却还要把它们重新糊回 parent timeline。

**Alternatives considered**

- 在现有 `SubRunThreadTree` 上继续打补丁：最终只会把新的 child session 模型再压回旧 read model。
- 完全不保留 parent summary 视图：会降低主会话的可决策性。

## Decision 7: `SubRunHandle` 要升级为 durable child-agent ref，而不是纯运行态句柄

**Decision**  
现有 `SubRunHandle` 需要向稳定 child-agent ref 演进，至少要稳定包含 `agent_id`、`session_id`、`sub_run_id`、`lineage_kind`、`parent_agent_id` 和当前状态来源。tool 结果、notification 和 server status 都以这份 ref 为主键，不再把它仅当成 live cancel handle。

**Rationale**  
当前句柄主要服务 spawn/cancel/status；一旦增加 send/wait/resume/close，句柄就必须在 durable history、server DTO、frontend state、runtime inbox 之间共享同一身份。

**Alternatives considered**

- 继续只用 `sub_run_id`：不够表达 child session 打开、resume 和 future fork。
- 只用 `session_id`：不够表达 parent-child lineage 与 agent 所有权。

## Decision 8: fork agent 不另起系统，只额外增加 lineage snapshot 元数据

**Decision**  
future fork agent 与普通 child agent 共享同一 child session 生命周期、协作工具族、notification 投影和 UI 模型；唯一新增的是 `lineage_kind=fork` 及其上下文快照元数据。

**Rationale**  
如果现在不把 spawn/fork 放进同一 lineage 模型，后续 fork 很容易变成第三套执行/展示/恢复语义，重复踩当前 subrun 的坑。

**Alternatives considered**

- 以后单独给 fork 做新 session 系统：会再次分裂工具、runtime 和 UI。
- 先完全不考虑 fork 元数据：会让当前 child session 模型在下一轮扩展时再次破裂。
