## MODIFIED Requirements

### Requirement: Kernel Owns Global Control Surface

`kernel` MUST 作为全局控制面承接 capability router、agent tree 和稳定控制合同。

#### Scenario: Application consumes stable control API

- **WHEN** `application` 编排 root agent、subrun、observe、close 或 route 请求
- **THEN** 它 SHALL 只依赖 `kernel` 暴露的稳定控制接口
- **AND** SHALL NOT 依赖 `agent_tree` 内部节点或内部状态容器

#### Scenario: Session truth remains outside kernel

- **WHEN** 系统推进某个 session 的 turn 或查询某个 session 的事件历史
- **THEN** `kernel` SHALL NOT 直接持有该 session 的真相聚合
- **AND** 这些职责 SHALL 继续由 `session-runtime` 承担

