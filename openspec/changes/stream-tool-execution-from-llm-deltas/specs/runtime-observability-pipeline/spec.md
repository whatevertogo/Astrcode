## ADDED Requirements

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

