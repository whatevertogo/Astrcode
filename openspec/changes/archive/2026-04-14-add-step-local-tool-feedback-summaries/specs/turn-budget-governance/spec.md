## ADDED Requirements

### Requirement: 工具反馈打包 SHALL 服从 prompt budget 与 clearable tool 语义

工具反馈打包 MUST 服从 turn 内的 prompt budget，并与现有 clearable tool / prune / micro compact 语义兼容，避免在引入反馈包后重新放大上下文噪音。

#### Scenario: budget 紧张时优先保留反馈包

- **WHEN** 一个 step 同时存在 feedback package 与大量可清理原始 tool result，且 prompt budget 紧张
- **THEN** request assembly SHALL 优先保留更高信息密度的 feedback package
- **AND** 对覆盖范围内的原始结果执行更积极的裁剪或清理

#### Scenario: clearable tool 结果可被反馈包替代

- **WHEN** 某批工具结果来自 clearable tool，且对应 feedback package 已生成
- **THEN** 系统 MAY 优先使用反馈包作为下一轮提示输入
- **AND** 原始结果继续保留为 durable 事实而不是 prompt 主体

#### Scenario: 未命中打包策略时保持原有路径

- **WHEN** 某个 step 未生成反馈包或反馈包不适用
- **THEN** 系统 SHALL 回退到当前原始 tool result + prune/micro compact 路径
- **AND** 不得因为未命中而丢失工具事实

