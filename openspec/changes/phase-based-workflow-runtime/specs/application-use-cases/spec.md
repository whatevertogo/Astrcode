## ADDED Requirements

### Requirement: `application` SHALL 在 session 提交入口编排 active workflow

`application` 在提交 session prompt 前 MUST 先解析当前 active workflow 与 current phase，再决定是否需要注入 phase overlay、解释用户信号、执行 phase 迁移，最后才编译治理面并委托 `session-runtime` 执行 turn。

#### Scenario: active workflow 为当前提交追加 phase overlay

- **WHEN** 当前 session 存在 active workflow，且当前 phase 为本轮提交生成额外 prompt declarations
- **THEN** `application` SHALL 把这些 declarations 通过现有 submission prompt path 注入
- **AND** SHALL NOT 绕过现有 governance surface / prompt assembly 标准路径

#### Scenario: 没有 active workflow 时保持现有 mode-only 提交流程

- **WHEN** 当前 session 没有 active workflow
- **THEN** `application` SHALL 继续沿用现有 mode/governance 提交流程
- **AND** SHALL NOT 要求上层调用方额外提供 workflow 参数才能完成一次普通提交

### Requirement: `application` SHALL 通过稳定 runtime 合同消费 workflow 所需事实

`application` 实现 workflow orchestration 时 MUST 通过 `session-runtime` 稳定 query / command 合同读取会话事实和推进 turn，而不是直接持有或篡改 runtime 内部状态结构。

#### Scenario: workflow approval 通过稳定入口触发 mode 迁移

- **WHEN** 某个 workflow signal 需要把 session 从一个 phase 迁移到绑定的下一个 mode
- **THEN** `application` SHALL 继续使用统一的 mode 切换入口完成迁移
- **AND** SHALL NOT 直接写入 `session-runtime` 内部 `current_mode` 或等价 shadow state

#### Scenario: workflow orchestration 读取 runtime authoritative facts

- **WHEN** `application` 需要判断当前 session 的 mode、phase、active tasks 或 child 状态
- **THEN** 它 SHALL 通过 `session-runtime` 暴露的稳定快照或 query 接口读取
- **AND** SHALL NOT 重新从原始 runtime 内部字段拼装同类真相
