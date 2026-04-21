## ADDED Requirements

### Requirement: governance reload SHALL treat mode catalog, capability surface, and skill catalog as one consistency unit

治理级 reload MUST 把 mode catalog、capability surface 与 skill catalog 视为同一个候选治理快照进行提交，而不是允许三者按各自顺序局部成功。成功时三者 SHALL 一起切换，失败时 SHALL 一起回滚到旧快照。

本要求与现有 `governance-reload-surface` 主 spec 中 “存在运行中 session 时拒绝 reload” 的约束并存：reload 只在无活跃 session 时触发，因此不存在 “running turn 用旧快照” 的场景。

#### Scenario: candidate governance snapshot commits all three registries together

- **WHEN** runtime reload 成功组装新的 plugin modes、external invokers 与 base skills，且无运行中 session
- **THEN** 系统 SHALL 以单次治理提交切换 mode catalog、capability surface 与 skill catalog
- **AND** 后续新 turn SHALL 看到同一版本的三类治理输入

#### Scenario: candidate governance snapshot rolls back completely on failure

- **WHEN** reload 过程中任一环节失败，例如 capability surface 校验失败
- **THEN** 系统 SHALL 恢复旧的 mode catalog、旧的 capability surface 与旧的 skill catalog
- **AND** SHALL NOT 留下”新 mode catalog + 旧 capability surface”或等价的部分更新状态

#### Scenario: reload emits diagnostics for governance snapshot version changes

- **WHEN** reload 成功切换到新的 mode catalog / capability surface / skill catalog
- **THEN** 系统 SHALL 记录可观测的版本边界或诊断信息
- **AND** 诊断结果 SHALL 能说明新快照包含哪些 mode、capability、skill 的变更
