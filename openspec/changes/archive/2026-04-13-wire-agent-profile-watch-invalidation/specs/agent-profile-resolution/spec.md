## MODIFIED Requirements

### Requirement: working-dir 级 agent profile 解析与缓存

`application` SHALL 提供基于 working-dir 的 agent profile 解析与缓存能力，并支持由 watch 事件驱动的显式失效，使后续执行读取新的 profile 结果。

#### Scenario: 首次读取目录 profile

- **WHEN** 某个 working-dir 首次请求 profile
- **THEN** 系统加载该目录对应的 profile 注册表并缓存结果

#### Scenario: 命中缓存

- **WHEN** 同一 working-dir 再次请求 profile
- **THEN** 系统复用缓存结果

#### Scenario: watch 事件触发 working-dir 失效

- **WHEN** 系统接收到该 working-dir agent 定义目录的变化事件
- **THEN** 系统 SHALL 失效对应缓存
- **AND** 后续解析 MUST 重新读取磁盘

#### Scenario: watch 事件触发全局失效

- **WHEN** 系统接收到全局 agent 定义目录的变化事件
- **THEN** 系统 SHALL 失效全局缓存
- **AND** 后续解析 MUST 重新读取新的全局 profile

### Requirement: profile 缓存不能替代业务校验

缓存 SHALL 只优化解析成本，不能跳过业务入口的存在性、权限和模式校验。

#### Scenario: 缓存命中但 agent 无效

- **WHEN** 缓存已存在，但请求的 agent 不在注册表内
- **THEN** 仍然返回业务错误
- **AND** 不因为命中缓存而直接继续执行

#### Scenario: 失效后后续执行以新结果为准

- **WHEN** 某个 cache 已因文件变化被失效
- **THEN** 后续执行 SHALL 使用重新解析得到的 profile
- **AND** MUST NOT 继续依赖失效前的旧结果
