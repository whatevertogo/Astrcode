## MODIFIED Requirements

### Requirement: `AgentControl` 迁入 `kernel/agent_tree`

`runtime-agent-control` SHALL 迁入 `kernel/agent_tree`，负责 lineage、subtree cancel/terminate、深度和并发约束。

#### Scenario: agent_tree 不依赖 runtime-config

- **WHEN** 检查 `kernel/agent_tree`
- **THEN** 不存在对 `astrcode_runtime_config` 的依赖

#### Scenario: 外部通过稳定 API 操作 agent_tree

- **WHEN** `application` 或 `session-runtime` 需要取消、观察或查询子执行
- **THEN** 通过 `kernel` 暴露的稳定 API 完成
- **AND** 不直接访问内部树结构
