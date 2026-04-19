## ADDED Requirements

### Requirement: Stable Subrun Status Contract

系统 SHALL 为 root agent 和 subrun 暴露稳定、一致、可查询的状态合同。

#### Scenario: Query root agent status

- **WHEN** 上层请求查询某个 root agent 或 session 关联的执行状态
- **THEN** 系统 SHALL 返回稳定状态视图
- **AND** 该视图 SHALL 不暴露 `agent_tree` 内部节点结构

#### Scenario: Query subrun status

- **WHEN** 上层请求查询某个 subrun 的当前状态
- **THEN** 系统 SHALL 返回统一状态视图
- **AND** 该视图 SHALL 覆盖运行中、已完成、已中断、失败等状态

#### Scenario: Status survives internal refactor

- **WHEN** `kernel` 内部 agent tree 存储结构发生变化
- **THEN** 上层依赖的状态查询合同 SHALL 保持稳定

