## ADDED Requirements

### Requirement: discovery SHALL 升级为统一资源发现面

系统的 discovery MUST 不再只覆盖 tools 与 skills，而是升级为统一资源发现面，至少覆盖 tools、commands、prompts、skills、themes 与其他 plugin 贡献的用户可见资源。

#### Scenario: 单次 discovery 返回多类资源候选
- **WHEN** 某个交互式 surface 请求当前可用候选
- **THEN** 系统 SHALL 能返回 tool、command、prompt、skill、theme 等多类候选
- **AND** 这些候选 SHALL 来自同一当前 active snapshot

### Requirement: discovery SHALL 由 plugin-host 驱动

统一资源发现 MUST 由 `plugin-host` 的当前生效 descriptor/snapshot 驱动，而不是由 server、CLI 或其他客户端各自维护平行目录。

#### Scenario: 新 plugin 资源在 reload 后被统一发现
- **WHEN** 某个 plugin 新增 prompt、skill 或 command 并成功 reload
- **THEN** 新资源 SHALL 自动进入统一 discovery 结果
- **AND** 客户端 SHALL 不需要维护独立的本地注册表
