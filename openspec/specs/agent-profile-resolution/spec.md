## Requirements

### Requirement: working-dir 级 agent profile 解析与缓存

`application` SHALL 提供基于 working-dir 的 agent profile 解析与缓存能力。

#### Scenario: 首次读取目录 profile

- **WHEN** 某个 working-dir 首次请求 profile
- **THEN** 系统加载该目录对应的 profile 注册表并缓存结果

#### Scenario: 命中缓存

- **WHEN** 同一 working-dir 再次请求 profile
- **THEN** 系统复用缓存结果

#### Scenario: 执行入口消费解析结果

- **WHEN** root execution 或 subagent execution 需要确定目标 agent
- **THEN** 系统 SHALL 使用该解析与缓存能力获取 profile
- **AND** MUST NOT 在编排层临时构造占位 profile 代替真实解析结果

---

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

---

### Requirement: Watch-driven invalidation does not rewrite running turns

系统 MUST 将 watch 驱动的 profile 更新限制在"后续解析可见"的边界内，而不是强行改写正在运行中的 turn。

#### Scenario: Running turn survives profile update

- **WHEN** profile 文件变化发生在某个 turn 执行期间
- **THEN** 当前 turn 继续使用其启动时的执行上下文
- **AND** 新 profile 仅对后续执行可见

---

### Requirement: profile 缓存不能替代业务校验

缓存 SHALL 只优化解析成本，不能跳过业务入口的存在性、权限和模式校验。

#### Scenario: 缓存命中但 agent 无效

- **WHEN** 缓存已存在，但请求的 agent 不在注册表内
- **THEN** 仍然返回业务错误
- **AND** 不因为命中缓存而直接继续执行

#### Scenario: 缓存命中但 mode 不允许

- **WHEN** 缓存命中，但目标 profile 不允许当前执行类型
- **THEN** 业务入口仍然返回显式校验错误
- **AND** MUST NOT 因缓存命中而跳过 mode 校验
