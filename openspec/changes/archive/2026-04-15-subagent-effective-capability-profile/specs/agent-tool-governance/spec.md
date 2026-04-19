## ADDED Requirements

### Requirement: spawn guidance MUST distinguish behavior template from task-scoped authorization

当 `spawn` 用于创建 child agent 时，协作 guidance MUST 明确 `profile` 用于选择行为模板，而 capability grant 用于限定本次任务授权。

#### Scenario: spawn prompt is rendered

- **WHEN** 系统向模型描述 `spawn`
- **THEN** prompt MUST 明确选 profile 是选行为模板
- **AND** MUST 明确 capability grant 才是本次任务能力范围的主要输入

#### Scenario: restricted child is launched

- **WHEN** child 以收缩后的 capability surface 启动
- **THEN** guidance MUST 明确 child 只会看到 launch-time resolved capability surface 允许的工具
- **AND** MUST NOT 把 profile 名称暗示成能力授权开关

### Requirement: spawn guidance SHALL keep reuse-first behavior under capability mismatch

引入 task-scoped capability grant 后，协作 prompt 仍 MUST 保持 reuse-first 协议，避免模型因为 capability 配置变细而默认回到过度 fan-out。

#### Scenario: existing idle child still matches the responsibility

- **WHEN** 当前已有一个 `Idle` child 拥有正确责任边界，且其生效工具集足以完成下一步工作
- **THEN** guidance MUST 继续优先推荐 `send`
- **AND** MUST NOT 因为 profile 或 grant 组合更多而默认再次 `spawn`

#### Scenario: existing child lacks required capability

- **WHEN** 当前 child 的生效工具集无法满足下一步工作
- **THEN** guidance MUST 允许通过新的 capability grant `spawn` 一个更合适的 child
- **AND** MUST 明确原因是 capability mismatch，而不是把 `Idle` 误表述成错误状态
