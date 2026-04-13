# governance-reload-surface Specification

## Purpose
TBD - created by archiving change unify-governance-and-reload-surface. Update Purpose after archive.
## Requirements
### Requirement: Governance reload SHALL unify all capability sources

系统 MUST 通过单一治理入口编排 builtin、MCP、plugin 三类能力来源的 reload，而不是允许各来源各自宣布“已刷新完成”。

#### Scenario: Unified reload rebuilds complete surface

- **WHEN** 上层触发治理级 reload
- **THEN** 系统 SHALL 重新读取生效配置并重建候选 capability surface
- **AND** 候选结果 SHALL 同时覆盖 builtin、MCP、plugin 三类能力来源

#### Scenario: Partial source refresh is not treated as full reload

- **WHEN** 仅有某个单独来源完成内部刷新但整份 surface 尚未一致替换
- **THEN** 系统 MUST NOT 将该结果暴露为完整 reload 成功

### Requirement: Governance reload SHALL replace surface atomically

系统 MUST 先完成候选 surface 组装，再一次性替换当前生效 surface；若组装失败，则保持旧 surface 继续服务。

#### Scenario: Candidate surface succeeds

- **WHEN** 候选 surface 组装成功
- **THEN** 系统 SHALL 一次性替换当前生效的 capability surface
- **AND** 治理快照 SHALL 与替换后的 surface 一致

#### Scenario: Candidate surface fails

- **WHEN** 候选 surface 组装或校验失败
- **THEN** 系统 SHALL 保留旧 surface 继续服务
- **AND** reload 结果 SHALL 显式返回失败原因

### Requirement: Governance reload SHALL reject active-session conflicts

系统 MUST 在存在运行中 session 时拒绝治理级 reload，避免执行中会话看到不一致的 capability surface。

#### Scenario: Reload rejected while sessions are running

- **WHEN** 存在运行中 session 且上层触发 reload
- **THEN** 系统 SHALL 返回显式业务错误
- **AND** 错误结果 SHALL 能指出 reload 因活动会话被拒绝

#### Scenario: Reload allowed when no active sessions remain

- **WHEN** 不存在运行中 session
- **THEN** 系统 SHALL 允许治理级 reload 继续执行

