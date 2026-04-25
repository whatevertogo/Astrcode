## Requirements

### Requirement: Discovery Uses Current Capability And Skill Facts

系统 SHALL 仅基于当前 capability surface、capability semantic model 与 skill catalog 提供工具、技能与 slash suggestion 发现能力。桌面端、浏览器端与消费 conversation surface 的终端前端 MUST 消费同一事实源，而不是各自维护平行 discovery cache、命令目录或本地 skill 注册表。

#### Scenario: Query tool discovery

- **WHEN** 上层请求查询工具能力或执行模糊搜索
- **THEN** 系统 SHALL 以当前 capability surface 为事实源
- **AND** SHALL NOT 依赖旧 runtime registry 或独立 discovery cache

#### Scenario: Query skill discovery

- **WHEN** 上层请求查询可用 skill 或 skill 语义信息
- **THEN** 系统 SHALL 以当前 skill catalog 为事实源
- **AND** SHALL NOT 绕过现有 catalog/materializer 链路

#### Scenario: Query slash suggestions

- **WHEN** 终端前端或其他交互式 surface 请求 slash command / skill suggestion
- **THEN** 系统 SHALL 基于当前 capability 与 skill 事实返回可用候选
- **AND** MUST NOT 让客户端自行拼出另一套脱离事实源的候选集合

### Requirement: Slash discovery SHALL 暴露可直接驱动终端命令面板的候选元数据

为支持 `/skill`、`/resume` 与其他 slash 工作流，discovery 结果 MUST 返回稳定的候选元数据，包括候选标识、显示标题、描述、匹配关键字，以及插入文本或可执行动作所需的信息；这些字段 MUST 能直接驱动终端命令面板，而不要求 CLI 硬编码展示规则。

#### Scenario: 查询终端 slash 候选

- **WHEN** 终端客户端按前缀或关键字请求 slash suggestion
- **THEN** 系统 SHALL 返回适合终端展示和选择的命令/skill 候选元数据
- **AND** 候选 MUST 能区分“插入文本”与“立即触发动作”这两类交互语义

#### Scenario: 不可用候选不会被暴露

- **WHEN** 某个 skill、命令或 capability 在当前 surface、当前 session 或当前权限下不可用
- **THEN** discovery SHALL 不返回该候选
- **AND** MUST NOT 让终端端先展示再在提交时才发现其必然失败

---

### Requirement: discovery SHALL 升级为统一资源发现面

系统的 discovery MUST 不再只覆盖 tools 与 skills，而是升级为统一资源发现面，至少覆盖 tools、commands、prompts、skills、themes 与其他 plugin 贡献的用户可见资源。

#### Scenario: 单次 discovery 返回多类资源候选
- **WHEN** 某个交互式 surface 请求当前可用候选
- **THEN** 系统 SHALL 能返回 tool、command、prompt、skill、theme 等多类候选
- **AND** 这些候选 SHALL 来自同一当前 active snapshot

---

### Requirement: discovery SHALL 由 plugin-host 驱动

统一资源发现 MUST 由 `plugin-host` 的当前生效 descriptor/snapshot 驱动，而不是由 server、CLI 或其他客户端各自维护平行目录。

#### Scenario: 新 plugin 资源在 reload 后被统一发现
- **WHEN** 某个 plugin 新增 prompt、skill 或 command 并成功 reload
- **THEN** 新资源 SHALL 自动进入统一 discovery 结果
- **AND** 客户端 SHALL 不需要维护独立的本地注册表
