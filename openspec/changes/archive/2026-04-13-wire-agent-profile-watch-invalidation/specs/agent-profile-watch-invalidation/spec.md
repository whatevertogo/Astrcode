## ADDED Requirements

### Requirement: Agent profile file changes invalidate execution-side caches

系统 SHALL 在 agent profile 定义文件变化时失效执行侧 profile 缓存，使后续 root/subagent 执行读取新的解析结果。

#### Scenario: Project-scoped agent definitions change

- **WHEN** 某个 working-dir 下的 `.astrcode/agents/` 发生文件变化
- **THEN** 系统失效该 working-dir 对应的 profile cache
- **AND** 后续执行重新从磁盘解析 profile

#### Scenario: Global agent definitions change

- **WHEN** 全局 agent 定义目录发生文件变化
- **THEN** 系统失效全局 profile cache
- **AND** 后续执行读取新的全局 profile 结果

#### Scenario: Unknown ownership falls back to safe invalidation

- **WHEN** 系统无法精确判断变化文件属于哪个 working-dir scope
- **THEN** 系统 SHALL 采用保守失效策略
- **AND** MUST NOT 继续依赖可能过期的 profile cache

### Requirement: Watch-driven invalidation does not rewrite running turns

系统 MUST 将 watch 驱动的 profile 更新限制在“后续解析可见”的边界内，而不是强行改写正在运行中的 turn。

#### Scenario: Running turn survives profile update

- **WHEN** profile 文件变化发生在某个 turn 执行期间
- **THEN** 当前 turn 继续使用其启动时的执行上下文
- **AND** 新 profile 仅对后续执行可见
