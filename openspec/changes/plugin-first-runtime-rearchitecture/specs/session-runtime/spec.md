## REMOVED Requirements

### Requirement: `session-runtime` 是唯一会话真相面

**Reason**: 本次重构将旧的大一统 `session-runtime` 拆解为 `agent-runtime` 与 `host-session` 两个 owner，分别负责最小 live runtime 与 durable session truth，不再保留单一 monolith crate 作为全部会话真相面。

**Migration**: 将 live turn/agent loop 相关实现迁入 `agent-runtime`；将事件日志、恢复、catalog、query/read model、branch/fork 迁入 `host-session`；删除旧 monolith `session-runtime` 的正式 owner 地位。

### Requirement: 会话执行构造逻辑归 `session-runtime`

**Reason**: turn 构造与执行循环将迁入新 `agent-runtime` crate；旧 `session-runtime` 不再继续作为 live runtime owner。

**Migration**: 将 `build_agent_loop`、`LoopRuntimeDeps`、`AgentLoop`、`TurnRunner` 等 live runtime 构造迁入 `agent-runtime`，由 `host-session` 负责驱动调用。
