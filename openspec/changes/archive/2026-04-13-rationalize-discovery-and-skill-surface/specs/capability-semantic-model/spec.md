## MODIFIED Requirements

### Requirement: Capability Semantic Model Supports Discovery Without Parallel Registries

能力语义模型 MUST 成为 discovery 能力的扩展点，而不是允许平行注册表再次出现。

#### Scenario: Discovery needs extra semantic metadata

- **WHEN** 工具发现需要标签、可见性、语义描述、搜索字段或排序字段
- **THEN** 系统 SHALL 在现有 capability semantic model 上扩展这些字段
- **AND** SHALL NOT 新建平行 discovery 模型作为第二事实源

