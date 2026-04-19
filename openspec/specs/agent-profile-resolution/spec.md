## Requirements

### Requirement: working-dir 级 agent profile 解析与缓存

`application` SHALL 提供基于 working-dir 的 agent profile 解析与缓存能力。

`ProfileResolutionService` 是核心服务，持有 `ProfileProvider`（trait，由 adapter-agents 实现）和两层缓存：
- `cache: DashMap<PathBuf, Arc<Vec<AgentProfile>>>` — working-dir 级 scoped 缓存
- `global_cache: RwLock<Option<Arc<Vec<AgentProfile>>>>` — 全局 profile 缓存

公开 API：
- `resolve(working_dir) -> Arc<Vec<AgentProfile>>` — 路径规范化后作为缓存键
- `resolve_global() -> Arc<Vec<AgentProfile>>`
- `find_profile(working_dir, profile_id) -> AgentProfile` — 内部调用 resolve 后按 id 匹配
- `find_global_profile(profile_id) -> AgentProfile`
- `invalidate(working_dir)` — 移除指定路径的缓存
- `invalidate_global()` — 清除所有 scoped cache + 全局 cache
- `invalidate_all()` — 清除全部

`ProfileProvider` trait（由 adapter-agents 实现）：
- `load_for_working_dir(working_dir) -> Vec<AgentProfile>`
- `load_global() -> Vec<AgentProfile>`

#### Scenario: 首次读取目录 profile

- **WHEN** 某个 working-dir 首次请求 profile（调用 `resolve`）
- **THEN** 系统通过 `ProfileProvider::load_for_working_dir` 加载该目录对应的 profile 注册表，缓存结果（DashMap），返回 `Arc<Vec<AgentProfile>>`

#### Scenario: 命中缓存

- **WHEN** 同一 working-dir（规范化后相同）再次请求 profile
- **THEN** 系统复用缓存结果，不再调用 provider

#### Scenario: 全局 profile 首次加载

- **WHEN** 首次调用 `resolve_global()`
- **THEN** 系统通过 `ProfileProvider::load_global` 加载全局 profiles，缓存（RwLock），返回 `Arc<Vec<AgentProfile>>`

#### Scenario: 全局缓存命中

- **WHEN** 再次调用 `resolve_global()` 且缓存非空
- **THEN** 直接返回缓存结果

#### Scenario: 按 ID 查找 scoped profile

- **WHEN** 调用 `find_profile(working_dir, profile_id)`
- **THEN** 系统先通过 `resolve` 获取列表，再按 id 匹配查找
- **AND** 未找到时返回 `ApplicationError::NotFound`（含 profile id 和 working dir）

#### Scenario: 按 ID 查找全局 profile

- **WHEN** 调用 `find_global_profile(profile_id)`
- **THEN** 系统通过 `resolve_global` 获取列表，按 id 匹配
- **AND** 未找到时返回 `ApplicationError::NotFound`（含 profile id）

#### Scenario: 执行入口消费解析结果

- **WHEN** root execution 或 subagent execution 需要确定目标 agent
- **THEN** 系统 SHALL 使用 `find_profile` 或 `find_global_profile` 获取 profile
- **AND** MUST NOT 在编排层临时构造占位 profile 代替真实解析结果

---

### Requirement: Agent profile file changes invalidate execution-side caches

系统 SHALL 在 agent profile 定义文件变化时失效执行侧 profile 缓存，使后续 root/subagent 执行读取新的解析结果。

#### Scenario: Project-scoped agent definitions change

- **WHEN** 某个 working-dir 下的 `.astrcode/agents/` 发生文件变化
- **THEN** 系统调用 `invalidate(working_dir)` 失效该 working-dir 对应的 profile cache
- **AND** 后续执行重新从磁盘解析 profile（调用 `load_for_working_dir`）

#### Scenario: Global agent definitions change

- **WHEN** 全局 agent 定义目录发生文件变化
- **THEN** 系统调用 `invalidate_global()` 失效全局 profile cache（同时清除所有 scoped cache）
- **AND** 后续执行读取新的全局 profile 结果

#### Scenario: Unknown ownership falls back to safe invalidation

- **WHEN** 系统无法精确判断变化文件属于哪个 working-dir scope
- **THEN** 系统 SHALL 调用 `invalidate_all()` 采用保守失效策略
- **AND** MUST NOT 继续依赖可能过期的 profile cache

#### Scenario: 已有快照不受失效影响

- **WHEN** 缓存失效后，已通过 `Arc` 分发的旧 profile 快照
- **THEN** 旧快照保持不变（Arc 不可变语义），新请求获得新快照

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
- **THEN** `find_profile` 仍然返回 `ApplicationError::NotFound`
- **AND** 不因为命中缓存而直接继续执行

#### Scenario: 缓存命中但 mode 不允许

- **WHEN** 缓存命中，但目标 profile 不允许当前执行类型
- **THEN** `ensure_profile_mode` 仍然返回显式校验错误
- **AND** MUST NOT 因缓存命中而跳过 mode 校验
