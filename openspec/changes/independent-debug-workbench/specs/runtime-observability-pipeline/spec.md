## MODIFIED Requirements

### Requirement: Runtime observability snapshots support debug time windows

runtime observability pipeline MUST 支持 Debug Workbench 读取最近时间窗口内的治理趋势样本，而不仅是单次瞬时快照。

#### Scenario: Debug window reopens after previous reads

- **WHEN** 开发者关闭并重新打开 Debug Workbench
- **THEN** 系统仍然可以返回最近时间窗口内的治理趋势样本
- **AND** 这些样本来自服务端维护的时间窗口快照
- **AND** 前端本地内存缓存不是唯一真相
