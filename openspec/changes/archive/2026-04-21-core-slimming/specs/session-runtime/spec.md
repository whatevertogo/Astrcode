## ADDED Requirements

### Requirement: `session-runtime` 拥有会话 durable projection 算法与快照

凡是依赖 session event 流恢复、服务于单 session authoritative read model 的 projection 算法与快照类型，`session-runtime` SHALL 作为唯一 owner。

这至少包括：

- input queue replay / projection 算法
- 其他需要根据 durable 事件重建的单 session 派生事实

#### Scenario: input queue replay is owned by session-runtime

- **WHEN** 检查 input queue 从 durable 事件恢复队列状态的实现
- **THEN** 该 replay / projection 算法 SHALL 位于 `session-runtime`
- **AND** `core` 不再保留等价的会话投影实现

#### Scenario: turn projection snapshot belongs to session-runtime

- **WHEN** 某个查询或恢复路径需要读取 turn projection 结果
- **THEN** projector、query、watcher 与等价的业务语义 SHALL 位于 `session-runtime`
- **AND** 若某个共享 checkpoint 载体暂时定义在 `core`，它也只作为跨 crate 合同存在，不改变 `session-runtime` 的业务 owner 地位

---

### Requirement: `session-runtime` 通过稳定端口消费副作用能力

当会话执行路径需要 durable tool result persist、项目目录解析或其他环境副作用时，`session-runtime` SHALL 通过稳定端口消费 adapter 提供的能力，而不是依赖 `core` 中的具体实现 helper。

#### Scenario: session-runtime does not reach into core for side effects

- **WHEN** 检查 `session-runtime` 中需要文件系统或 durable persist 的路径
- **THEN** 它们 SHALL 通过 port trait 调用外部能力
- **AND** 不再依赖 `core` 内的具体 IO / shell helper
