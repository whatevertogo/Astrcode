## ADDED Requirements

### Requirement: Discovery Uses Current Capability And Skill Facts

系统 SHALL 仅基于当前 capability surface、capability semantic model 与 skill catalog 提供工具与技能发现能力。

#### Scenario: Query tool discovery

- **WHEN** 上层请求查询工具能力或执行模糊搜索
- **THEN** 系统 SHALL 以当前 capability surface 为事实源
- **AND** SHALL NOT 依赖旧 runtime registry 或独立 discovery cache

#### Scenario: Query skill discovery

- **WHEN** 上层请求查询可用 skill 或 skill 语义信息
- **THEN** 系统 SHALL 以当前 skill catalog 为事实源
- **AND** SHALL NOT 绕过现有 catalog/materializer 链路

#### Scenario: Discovery can be explicitly removed

- **WHEN** 某个旧 discovery 接口不再具备产品价值
- **THEN** 系统 MAY 明确废弃并删除该能力
- **AND** SHALL NOT 保留空实现或兼容性 skeleton

