## ADDED Requirements

### Requirement: subrun status SHALL expose launch-time resolved capability snapshots

系统 MUST 为 subrun 暴露 child 启动时已经求得的 resolved capability snapshot，避免调用方只能从 transcript 或最新配置反推 child capability。

#### Scenario: query running child status

- **WHEN** 上层查询一个运行中的 subrun 状态
- **THEN** 返回结果 MUST 包含该 child 启动时的 `resolved_limits`
- **AND** 这些 limits MUST 反映 child 的 launch-time capability surface，而不是当前全局 capability registry 的完整视图

#### Scenario: query completed child status

- **WHEN** 上层查询一个已经完成的 subrun 状态
- **THEN** 返回结果 MUST 仍然保留该 child 启动时的 `resolved_limits`
- **AND** 调用方 MUST 能据此解释该 child 为什么在运行期间能或不能调用某些工具

### Requirement: subrun lifecycle events SHALL persist launch-time capability projections

subrun 的生命周期 durable 事件 MUST 持久化 child 启动时的 capability projection，确保状态查询与回放不依赖临时内存重新计算。

#### Scenario: child launch is recorded

- **WHEN** 系统记录某个 child 的 `SubRunStarted` 事件
- **THEN** 该事件 MUST 包含 child 的 `resolved_limits`
- **AND** 这些 limits MUST 来源于 child 启动前已经完成的 capability projection

#### Scenario: status is rebuilt from durable history

- **WHEN** 系统基于 durable 事件重建 subrun 状态
- **THEN** 返回的 `resolved_limits` MUST 可从 lifecycle 事件恢复
- **AND** MUST NOT 依赖当前磁盘上的最新 profile 或最新 capability router 重新推断
