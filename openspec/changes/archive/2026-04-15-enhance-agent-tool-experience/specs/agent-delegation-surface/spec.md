## ADDED Requirements

### Requirement: model-visible delegation catalog SHALL expose behavior templates, not capability authority

当当前 session 可使用 child delegation 时，系统 MUST 提供一个面向模型的 delegation catalog，并且该 catalog 只能基于当前可作为 child 使用的 behavior template 生成；它 MUST NOT 把 `AgentProfile` 伪装成 capability 授权目录。

#### Scenario: render available child templates

- **WHEN** prompt builder 为具备 child delegation 能力的 session 组装 delegation surface
- **THEN** 系统 MUST 只展示当前可作为 child 使用的 behavior template
- **AND** 每个 entry MUST 包含足以帮助选择的行为模板摘要或用途说明

#### Scenario: hide unavailable child templates

- **WHEN** 某个 profile 当前不允许作为 child，或当前 runtime / policy 不允许模型使用对应 delegation 能力
- **THEN** 该 profile MUST NOT 出现在 delegation catalog 中
- **AND** 系统 MUST NOT 指望 runtime 在模型选择之后再去纠正一个本可提前隐藏的 entry

#### Scenario: catalog does not claim per-profile tool ownership

- **WHEN** 系统渲染 delegation catalog
- **THEN** catalog MUST NOT 把某个 behavior template 表达成一组静态工具权限
- **AND** MUST 保持“profile 是行为模板，capability truth 在 launch 时求解”的边界

### Requirement: child execution contract SHALL be rendered through a child-scoped prompt surface

系统 MUST 为 child agent 渲染独立的 execution contract prompt surface，用来明确责任边界、交付方式与限制条件，而不是要求调用方仅靠自然语言 prompt 自行约定这些信息。

#### Scenario: fresh child receives full execution contract

- **WHEN** 系统首次启动一个承担新责任边界的 child
- **THEN** child prompt MUST 包含该责任边界、期望交付形式与回传摘要要求
- **AND** 这些信息 MUST 作为 child-scoped contract surface 出现，而不是散落在工具 description 中

#### Scenario: resumed child receives delta-oriented execution contract

- **WHEN** 父级复用已有 child 并发送下一步任务
- **THEN** child prompt MUST 保留既有 responsibility continuity
- **AND** 新增 prompt 内容 MUST 以具体 delta instruction 为主，而不是重新灌入完整 fresh briefing

#### Scenario: restricted child receives capability-aware contract

- **WHEN** child 以收缩后的 capability surface 启动
- **THEN** child execution contract MUST 明确暴露本次 capability limit 的摘要
- **AND** MUST 明确 child 不应承担超出该 capability surface 的工作
