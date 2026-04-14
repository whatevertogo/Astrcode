# runtime-observability-pipeline Specification

## Purpose

定义运行时 observability 管线的稳定行为，包括实时采集器、读/执行路径指标覆盖、协作诊断以及 Debug Workbench 时间窗口支持。

## Requirements

### Requirement: Runtime observability SHALL be backed by live collectors

系统 MUST 使用真实运行时采集器生成 observability 快照，而不是使用默认零值占位实现。

#### Scenario: Governance snapshot exposes live metrics

- **WHEN** 上层读取治理快照
- **THEN** 返回的 observability 数据 SHALL 来自实时采集器
- **AND** MUST NOT 以固定零值伪装为真实结果

#### Scenario: Missing metric source is explicit

- **WHEN** 某类指标来源尚未接线或暂时不可用
- **THEN** 系统 SHALL 显式处理该状态
- **AND** MUST NOT 静默返回误导性的成功零值

### Requirement: Runtime observability SHALL cover read and execution paths

系统 MUST 同时采集读路径与执行路径的关键指标，包括 session rehydrate、SSE catch-up、turn execution、subrun execution、delivery diagnostics 以及 agent collaboration diagnostics。

#### Scenario: Read path metrics are recorded

- **WHEN** 系统执行 session 重水合或 SSE 回放
- **THEN** 对应 observability 指标 SHALL 被记录

#### Scenario: Execution path metrics are recorded

- **WHEN** 系统执行 turn、subrun、delivery 或 agent collaboration 相关流程
- **THEN** 对应 observability 指标 SHALL 被记录
- **AND** 失败路径同样 SHALL 被统计

#### Scenario: Collaboration diagnostics are exposed

- **WHEN** 上层读取治理快照或等价 observability 读模型
- **THEN** 返回结果 SHALL 包含 agent collaboration 诊断
- **AND** 该诊断 SHALL 能区分 spawn、send、observe、close、delivery 与拒绝/失败路径

### Requirement: Runtime observability snapshots support debug time windows

runtime observability pipeline MUST 支持 Debug Workbench 读取最近时间窗口内的治理趋势样本，而不仅是单次瞬时快照。

#### Scenario: Debug window reopens after previous reads

- **WHEN** 开发者关闭并重新打开 Debug Workbench
- **THEN** 系统仍然可以返回最近时间窗口内的治理趋势样本
- **AND** 这些样本来自服务端维护的时间窗口快照
- **AND** 前端本地内存缓存不是唯一真相

### Requirement: runtime observability SHALL 覆盖流式 tool 调度诊断

运行时 observability MUST 覆盖流式 tool 调度带来的关键诊断数据，包括提前执行次数、被保守回退的次数以及 LLM/tool 的重叠执行情况，从而判断该优化是否真正生效。

#### Scenario: 记录提前执行次数

- **WHEN** 系统在流式阶段提前启动一个安全工具调用
- **THEN** 对应 observability 指标 SHALL 被记录

#### Scenario: 记录保守回退原因

- **WHEN** 某个流式工具调用因为参数未闭合或存在副作用而未被提前执行
- **THEN** 系统 SHALL 记录该回退原因
- **AND** 该信息 SHALL 能被诊断读取

#### Scenario: 记录 LLM/tool 重叠执行

- **WHEN** 某个 step 内存在 LLM streaming 与工具执行重叠的时间窗口
- **THEN** 运行时 observability SHALL 记录该重叠信息
- **AND** 失败或取消路径同样 SHALL 被统计
