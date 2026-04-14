## ADDED Requirements

### Requirement: Debug Workbench Runtime Overview

系统 MUST 提供一个面向 Debug Workbench 的 runtime overview 读模型，用于展示全局治理指标和执行诊断。

#### Scenario: Read runtime overview

- **WHEN** Debug Workbench 调用 `GET /api/debug/runtime/overview`
- **THEN** 系统返回当前 runtime 的调试概览
- **AND** 返回值包含 `spawn_rejection_ratio_bps`、`observe_to_action_ratio_bps`、`child_reuse_ratio_bps`
- **AND** 返回值包含现有 delivery latency、orphan child、spawn-to-delivery 等聚合值

### Requirement: Debug Workbench Timeline Window

系统 MUST 提供由服务端维护的最近时间窗口趋势样本，供 Debug Workbench 读取。

#### Scenario: Read recent 5 minute timeline

- **WHEN** Debug Workbench 调用 `GET /api/debug/runtime/timeline`
- **THEN** 系统返回最近 5 分钟时间窗口内的样本序列
- **AND** 每个样本都包含时间戳、`spawn_rejection_ratio_bps`、`observe_to_action_ratio_bps`、`child_reuse_ratio_bps`
- **AND** 过期窗口外的样本不会继续返回

### Requirement: Debug Workbench Session Trace

系统 MUST 提供会话级 debug trace 查询，用于查看当前会话及其相关 agent 的近期调试流。

#### Scenario: Read session debug trace

- **WHEN** Debug Workbench 调用 `GET /api/debug/sessions/{id}/trace`
- **THEN** 系统返回该 session 的调试 trace
- **AND** trace 按时间倒序或显式时间顺序稳定输出
- **AND** trace 至少包含 tool call、collaboration fact 与 delivery 关键事件的聚合结果
- **AND** 其它 session 的事件不会串入该响应

### Requirement: Debug Workbench Session Agent Tree

系统 MUST 提供 session 下的 child agent tree / lineage 查询。

#### Scenario: Read session agent lineage

- **WHEN** Debug Workbench 调用 `GET /api/debug/sessions/{id}/agents`
- **THEN** 系统返回该 session 相关的 child agent tree / lineage 摘要
- **AND** 每个节点包含 parent/child 关系与 lifecycle 状态
- **AND** 返回值可以被前端用于构建 agent tree 视图
