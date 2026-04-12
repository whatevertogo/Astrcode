## ADDED Requirements

### Requirement: working-dir 级 agent profile 解析与缓存

`application` SHALL 提供基于 working-dir 的 agent profile 解析与缓存能力。

#### Scenario: 首次读取目录 profile
- **WHEN** 某个 working-dir 首次请求 profile
- **THEN** 系统加载该目录对应的 profile 注册表并缓存结果

#### Scenario: 命中缓存
- **WHEN** 同一 working-dir 再次请求 profile
- **THEN** 系统复用缓存结果

### Requirement: profile 缓存不能替代业务校验

缓存 SHALL 只优化解析成本，不能跳过业务入口的存在性、权限和模式校验。

#### Scenario: 缓存命中但 agent 无效
- **WHEN** 缓存已存在，但请求的 agent 不在注册表内
- **THEN** 仍然返回业务错误
- **AND** 不因为命中缓存而直接继续执行
