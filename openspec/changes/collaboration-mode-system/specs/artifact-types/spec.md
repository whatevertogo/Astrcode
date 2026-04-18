## ADDED Requirements

### Requirement: ModeArtifactRef 轻量引用
系统 SHALL 定义 `ModeArtifactRef` 结构体，包含：
- `artifact_id`: String
- `source_mode`: String（产出模式 ID）
- `kind`: String（artifact 类型）
- `status`: ArtifactStatus（Draft | Accepted | Rejected | Superseded）
- `summary`: String（人可读摘要）

#### Scenario: ModeArtifactRef 可序列化为 JSON
- **WHEN** 一个 ModeArtifactRef 被序列化
- **THEN** JSON 包含 artifactId、sourceMode、kind、status、summary 字段

#### Scenario: ModeArtifactRef 可嵌入 StorageEvent
- **WHEN** artifact 创建或状态变更
- **THEN** ModeArtifactRef 作为 StorageEvent payload 的一部分被持久化

### Requirement: ModeArtifactBody 完整负载
系统 SHALL 定义 `ModeArtifactBody` 枚举：
- `Plan(PlanContent)`: 方案内容
- `Review(ReviewContent)`: 审查内容
- `Custom { schema_id, schema_version, data: Value }`: SDK 扩展

#### Scenario: Plan body 包含结构化步骤
- **WHEN** Plan(PlanContent) 被构造
- **THEN** PlanContent 包含 steps、assumptions、open_questions、touched_paths、risk_notes 字段

#### Scenario: Custom body 携带 schema 信息
- **WHEN** Custom body 被构造
- **THEN** 包含 schema_id（标识内容格式）、schema_version、data（自由 JSON）

### Requirement: PlanContent 强类型方案结构
系统 SHALL 定义 `PlanContent` 结构体，包含：
- `steps: Vec<PlanStep>`（步骤列表）
- `assumptions: Vec<String>`（假设条件）
- `open_questions: Vec<String>`（待确认问题）
- `touched_paths: Vec<String>`（涉及文件路径）
- `risk_notes: Vec<String>`（风险说明）

#### Scenario: PlanStep 包含描述和风险等级
- **WHEN** PlanStep 被构造
- **THEN** 包含 description 字段

### Requirement: ArtifactStatus 状态枚举
系统 SHALL 定义 `ArtifactStatus` 枚举：Draft | Accepted | Rejected | Superseded。

#### Scenario: 新建 artifact 默认 Draft
- **WHEN** artifact 被创建
- **THEN** status 为 Draft

#### Scenario: 状态转换是单向的
- **WHEN** artifact 从 Draft 转为 Accepted
- **THEN** 后续不能再改回 Draft
