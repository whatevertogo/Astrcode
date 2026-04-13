## Why

当前仓库已经迁出了可观测性快照类型，但真实指标采集和治理接线没有跟上：server 侧仍使用全零默认快照，导致运行时状态接口在结构上存在、在语义上失真。若不先恢复这条链路，后续排障、回归分析和治理决策都会建立在错误事实之上。

## What Changes

- 恢复新架构下的真实 runtime observability 管线，使会话重水合、SSE catch-up、turn 执行、subrun 执行和交付缓冲等指标能够持续采集并汇总。
- 建立治理可消费的稳定 observability 能力，确保 server 暴露的治理快照来自真实采集器，而不是占位默认值。
- 调整 turn 级汇总与全局 observability 之间的衔接方式，让原始事件、turn 聚合结果和治理快照各自承担清晰职责。
- 保持现有 DTO 语义稳定，优先补齐采集与接线，而不是重新设计一套新的指标协议。

## Non-Goals

- 不在本次 change 中设计新的前端图表、监控面板或外部 metrics 导出协议。
- 不在本次 change 中扩大日志系统范围来替代 observability 指标。
- 不在本次 change 中改写 turn 主流程的业务语义，除非为了稳定采集指标必须做局部收口。

## Capabilities

### New Capabilities

- `runtime-observability-pipeline`: 定义跨 `session-runtime`、`application`、`server` 的指标采集、聚合和治理快照接线要求。

### Modified Capabilities

- `turn-observability`: 扩展 turn 汇总与运行时 observability 管线之间的契约，要求治理读取稳定聚合结果而不是空快照。

## Impact

- 受影响代码主要位于 `crates/session-runtime/src/turn/*`、`crates/application/src/observability/*`、`crates/application/src/lifecycle/*`、`crates/server/src/bootstrap/governance.rs`、`crates/server/src/http/mapper.rs` 与相关测试。
- 用户可见影响：运行时状态、诊断和排障结果将反映真实执行情况，避免“指标全为 0”或缺失关键计数。
- 开发者可见影响：测试需要从“DTO 类型存在”升级到“采集链路真实生效”，并覆盖 session rehydrate、SSE 回放、turn/subrun 完成与失败路径。
- 迁移与回滚思路：优先在现有 DTO 上补齐采集器接线；若新采集链路存在局部缺口，回滚应保留旧字段但显式禁用对应指标来源，避免静默返回误导性零值。
