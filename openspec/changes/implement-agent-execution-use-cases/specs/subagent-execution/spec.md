## ADDED Requirements

### Requirement: `application` 提供子代理执行入口

`application` SHALL 提供正式的子代理执行入口，负责创建子执行、协调全局控制并触发单 session turn。

#### Scenario: spawn 子代理
- **WHEN** 调用子代理执行入口并提供必要上下文
- **THEN** 系统创建子执行并返回可追踪结果

#### Scenario: 子代理完成后结果回流父级
- **WHEN** 子代理执行结束
- **THEN** 结果通过既有 delivery / control 机制回流父级
- **AND** 不在 `application` 内形成新的结果真相缓存

### Requirement: 子代理关闭与观察走稳定业务入口

子代理的关闭和观察 SHALL 通过稳定入口访问，而不是让路由层直接拼接底层对象。

#### Scenario: 关闭子代理
- **WHEN** 调用关闭入口
- **THEN** 业务入口协调 control/session 两侧完成关闭

#### Scenario: 查询子代理状态
- **WHEN** 调用观察入口
- **THEN** 返回与当前 control 真相一致的状态快照
