## ADDED Requirements

### Requirement: SessionState 管理 active_artifacts
系统 SHALL 在 `SessionState` 中新增 `active_artifacts: StdMutex<Vec<ModeArtifactRef>>` 字段，跟踪当前会话的活跃 artifact。

#### Scenario: artifact 创建后加入 active_artifacts
- **WHEN** Plan 模式产出一个新的 ModeArtifact
- **THEN** 其 ModeArtifactRef 被添加到 active_artifacts

#### Scenario: artifact 状态变更同步更新
- **WHEN** 一个 active artifact 从 Draft 变为 Accepted
- **THEN** active_artifacts 中对应的 ref 的 status 更新为 Accepted

#### Scenario: artifact 被 Superseded 后保留但标记
- **WHEN** 新 plan 产出后旧 plan 被标记为 Superseded
- **THEN** 旧 plan 仍在 active_artifacts 中，但 status 为 Superseded

### Requirement: Artifact 变更通过 StorageEvent 持久化
系统 SHALL 在 `StorageEventPayload` 中新增以下变体：
- `ModeArtifactCreated { ref: ModeArtifactRef, body: ModeArtifactBody, timestamp }`
- `ModeArtifactStatusChanged { artifact_id, from_status, to_status, timestamp }`

#### Scenario: artifact 创建产生事件
- **WHEN** Plan 模式产出 ModeArtifact
- **THEN** 一条 ModeArtifactCreated 事件被持久化

#### Scenario: artifact 状态变更产生事件
- **WHEN** 用户接受一个 plan artifact
- **THEN** 一条 ModeArtifactStatusChanged { from: Draft, to: Accepted } 事件被持久化

#### Scenario: 旧会话 replay 不受影响
- **WHEN** replay 不包含 artifact 事件的旧会话
- **THEN** active_artifacts 为空列表，不报错
